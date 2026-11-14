//! «Полка»: мост между экранами и ядром `polka-core`.
//!
//! Экраны получают список записей и после каждого изменения — изменённую запись.
//! Поиск в каталогах и скачивание обложек идут в фоновом потоке, чтобы окно не замирало.
//! Снимки обложек отдаёт протокол `cover` (адрес `cover://localhost/<id>` или
//! `http://cover.localhost/<id>` на Windows и Android) — так экран не гоняет картинки через команды.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use polka_core::catalog::{self, http::Net, Candidate, Found, Keys};
use polka_core::db::{sniff_image, Store};
use polka_core::model::{Format, Status, Title, KIND};
use polka_core::stats::{self, Goal, YearStats, GOAL_KIND};
use polka_core::{new_id, Date};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::{Manager, State};

mod backup;
mod cloud;
mod lan;

/// Версия «Полки» — пишется в резервные копии.
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

struct AppState {
    /// База — общая с потоками обмена по Wi-Fi.
    store: Arc<Mutex<Store>>,
    dir: PathBuf,
    keys: Mutex<Keys>,
    net: Arc<Net>,
    /// Номер этого устройства в журнале.
    id: String,
    /// Как называть это устройство в копиях и при связи: имя компьютера или «Телефон».
    name: String,
    /// Связь по Wi-Fi: код, связанные устройства, кто рядом.
    lan: lan::Lan,
}

impl AppState {
    fn store(&self) -> Res<MutexGuard<'_, Store>> {
        self.store.lock().map_err(|_| "база занята — попробуйте ещё раз".to_string())
    }

    fn keys(&self) -> Keys {
        self.keys.lock().map(|k| k.clone()).unwrap_or_default()
    }
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn day(s: &str) -> Res<Date> {
    Date::parse(s).ok_or_else(|| format!("неверная дата «{s}»"))
}

// ——— Устройство, ключ базы и ключи каталогов ———

/// Имя устройства для копий: имя компьютера в системе, на телефоне — «Телефон».
fn device_name() -> String {
    if cfg!(mobile) {
        return "Телефон".into();
    }
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default();
    let name: String = raw.trim().chars().filter(|c| !c.is_control()).take(40).collect();
    if name.is_empty() {
        "Компьютер".into()
    } else {
        name
    }
}

/// Смещение местного времени от UTC в минутах — его присылает экран (для имён копий).
pub(crate) fn tz_minutes(st: &Store) -> i64 {
    st.meta("tz").ok().flatten().and_then(|v| v.parse::<i64>().ok()).filter(|m| m.abs() <= 14 * 60).unwrap_or(0)
}

#[tauri::command]
fn set_tz(state: State<'_, AppState>, minutes: i32) -> Res<()> {
    if minutes.abs() > 14 * 60 {
        return Err("неверный часовой пояс".into());
    }
    state.store()?.set_meta("tz", &minutes.to_string()).map_err(err)
}

/// Общие настройки — записи вида `setting`: они попадают в журнал и доходят до других устройств.
const SETTING_KIND: &str = "setting";

pub(crate) fn setting(st: &Store, key: &str) -> Option<String> {
    st.get::<String>(SETTING_KIND, key).ok().flatten()
}

pub(crate) fn put_setting(st: &mut Store, key: &str, value: &str) -> Res<()> {
    st.put(SETTING_KIND, key, &value.to_string()).map_err(err)
}

#[derive(Serialize, Deserialize)]
struct Identity {
    id: String,
}

fn load_identity(dir: &Path) -> Res<Identity> {
    let path = dir.join("device.json");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(id) = serde_json::from_str::<Identity>(&text) {
            if !id.id.is_empty() {
                return Ok(id);
            }
        }
    }
    let id = Identity { id: new_id().map_err(err)? };
    write_atomic(&path, serde_json::to_string_pretty(&id).map_err(err)?.as_bytes())?;
    Ok(id)
}

fn write_atomic(path: &Path, data: &[u8]) -> Res<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

/// Секрет на диске: на Windows и Android — зашифрованный средствами системы (DPAPI, Android Keystore),
/// на Linux (разработка) — как есть.
fn read_secret(dir: &Path, name: &str) -> Res<Option<Vec<u8>>> {
    let path = dir.join(format!("{name}.sec"));
    match std::fs::read(&path) {
        Ok(blob) => polka_secure::unprotect(&blob).map(Some).map_err(|e| format!("{name}: не достать из хранилища системы: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(e)),
    }
}

fn write_secret(dir: &Path, name: &str, data: &[u8]) -> Res<()> {
    let blob = polka_secure::protect(data).map_err(err)?;
    write_atomic(&dir.join(format!("{name}.sec")), &blob)
}

/// Ключ базы. Новый создаётся только для новой базы: если база есть, а ключа нет, —
/// это ошибка, а не повод начать с пустой полки.
fn load_key(dir: &Path, db: &Path) -> Res<[u8; 32]> {
    if let Some(k) = read_secret(dir, "key")? {
        return <[u8; 32]>::try_from(k.as_slice()).map_err(|_| "ключ базы повреждён".to_string());
    }
    if db.exists() {
        return Err("база есть, а ключа к ней нет — восстановите файл key.sec или резервную копию".into());
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(err)?;
    write_secret(dir, "key", &key)?;
    Ok(key)
}

#[derive(Default, Serialize, Deserialize)]
struct StoredKeys {
    #[serde(default)]
    poiskkino: Option<String>,
    #[serde(default)]
    google: Option<String>,
}

fn load_keys(dir: &Path) -> Keys {
    let stored: StoredKeys = read_secret(dir, "catalog-keys").ok().flatten().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    Keys { poiskkino: stored.poiskkino, google: stored.google }
}

fn save_keys(dir: &Path, keys: &Keys) -> Res<()> {
    let stored = StoredKeys { poiskkino: keys.poiskkino.clone(), google: keys.google.clone() };
    write_secret(dir, "catalog-keys", &serde_json::to_vec(&stored).map_err(err)?)
}

// ——— Записи ———

fn get_title(st: &Store, id: &str) -> Res<Title> {
    st.get::<Title>(KIND, id).map_err(err)?.ok_or_else(|| "запись не найдена — возможно, её удалили".to_string())
}

fn put_title(st: &mut Store, mut t: Title) -> Res<Title> {
    t.tidy();
    t.creators = t.creators.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    t.check().map_err(err)?;
    st.put(KIND, &t.id, &t).map_err(err)?;
    Ok(t)
}

#[tauri::command]
fn titles(state: State<'_, AppState>) -> Res<Vec<Title>> {
    state.store()?.list::<Title>(KIND).map_err(err)
}

/// Записать изменённую запись (или новую без номера). Обложку экран не меняет — для неё свои команды.
#[tauri::command]
fn save_title(state: State<'_, AppState>, title: Title, today: String) -> Res<Title> {
    let mut st = state.store()?;
    let mut t = title;
    if t.id.is_empty() {
        t.id = new_id().map_err(err)?;
        t.added = day(&today)?;
        t.cover = None;
        t.cover_source = None;
    } else if let Some(old) = st.get::<Title>(KIND, &t.id).map_err(err)? {
        t.cover = old.cover;
        t.cover_source = old.cover_source;
        t.added = old.added;
    }
    put_title(&mut st, t)
}

#[tauri::command]
fn delete_title(state: State<'_, AppState>, id: String) -> Res<()> {
    let mut st = state.store()?;
    get_title(&st, &id)?;
    st.delete(KIND, &id).map_err(err)
}

#[tauri::command]
fn set_status(state: State<'_, AppState>, id: String, status: Status, today: String) -> Res<Title> {
    let mut st = state.store()?;
    let mut t = get_title(&st, &id)?;
    t.set_status(status, day(&today)?);
    put_title(&mut st, t)
}

#[tauri::command]
fn mark_episode(state: State<'_, AppState>, id: String, season: u32, ep: u32, seen: bool, today: String) -> Res<Title> {
    let mut st = state.store()?;
    let mut t = get_title(&st, &id)?;
    t.mark_episode(season, ep, seen, day(&today)?).map_err(err)?;
    put_title(&mut st, t)
}

#[tauri::command]
fn mark_season(state: State<'_, AppState>, id: String, season: u32, seen: bool, today: String) -> Res<Title> {
    let mut st = state.store()?;
    let mut t = get_title(&st, &id)?;
    t.mark_season(season, seen, day(&today)?).map_err(err)?;
    put_title(&mut st, t)
}

// ——— Каталоги ———

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

#[tauri::command]
async fn search(state: State<'_, AppState>, kind: String, query: String) -> Res<Found> {
    let net = state.net.clone();
    let keys = state.keys();
    blocking(move || {
        Ok(match kind.as_str() {
            "film" => catalog::search_films(net.as_ref(), &query, &keys),
            _ => catalog::search_books(net.as_ref(), &query, &keys),
        })
    })
    .await
}

/// Добавление по ссылке — или по тексту из «Поделиться», в котором есть ссылка.
#[tauri::command]
async fn from_link(state: State<'_, AppState>, url: String) -> Res<Candidate> {
    let net = state.net.clone();
    let keys = state.keys();
    blocking(move || {
        let url = catalog::link::link_in(&url).unwrap_or(url.trim()).to_string();
        catalog::from_link(net.as_ref(), &url, &keys).map_err(err)
    })
    .await
}

#[tauri::command]
async fn details(state: State<'_, AppState>, candidate: Candidate) -> Res<Candidate> {
    let net = state.net.clone();
    let keys = state.keys();
    blocking(move || Ok(catalog::details(net.as_ref(), &candidate, &keys))).await
}

#[derive(Deserialize)]
struct AddForm {
    status: Status,
    date: String,
    #[serde(default)]
    rating: Option<u8>,
    #[serde(default)]
    format: Option<Format>,
    #[serde(default)]
    tags: Vec<String>,
}

/// Новая запись из найденного. Обложка скачивается сейчас; если не скачалась —
/// запись сохраняется без неё (экран предложит выбрать другую).
#[tauri::command]
async fn add_title(state: State<'_, AppState>, candidate: Candidate, cover_url: Option<String>, cover_label: Option<String>, form: AddForm, today: String) -> Res<Title> {
    let today = day(&today)?;
    let on = day(&form.date)?;
    let net = state.net.clone();
    let cover = match cover_url {
        Some(url) => blocking(move || Ok(catalog::fetch_cover(net.as_ref(), &url).ok())).await?,
        None => None,
    };
    let mut t = candidate.to_title(today);
    t.id = new_id().map_err(err)?;
    t.set_status(form.status, on);
    if matches!(form.status, Status::Done | Status::Dropped) {
        t.rating = form.rating.filter(|r| (1..=10).contains(r));
    }
    t.format = if t.kind == polka_core::model::Kind::Book { form.format.or(Some(Format::Paper)) } else { None };
    t.tags = form.tags;
    let mut st = state.store()?;
    if let Some((mime, data)) = cover {
        t.cover = Some(st.put_blob(&mime, &data).map_err(err)?);
        t.cover_source = cover_label.filter(|l| !l.trim().is_empty());
    }
    put_title(&mut st, t)
}

#[tauri::command]
async fn set_cover_url(state: State<'_, AppState>, id: String, url: String, label: String) -> Res<Title> {
    let net = state.net.clone();
    let (mime, data) = blocking(move || catalog::fetch_cover(net.as_ref(), &url).map_err(err)).await?;
    let mut st = state.store()?;
    let mut t = get_title(&st, &id)?;
    t.cover = Some(st.put_blob(&mime, &data).map_err(err)?);
    t.cover_source = Some(label).filter(|l| !l.trim().is_empty());
    put_title(&mut st, t)
}

/// Снимок из data URL (`data:image/jpeg;base64,…`): тип проверяется по содержимому.
fn decode_data_url(data_url: &str) -> Res<(String, Vec<u8>)> {
    let (head, body) = data_url.split_once(',').ok_or("снимок не прочитался")?;
    if !head.starts_with("data:image/") || !head.ends_with(";base64") {
        return Err("подходят только снимки JPEG, PNG и WebP".into());
    }
    let data = B64.decode(body.trim()).map_err(|_| "снимок повреждён".to_string())?;
    let mime = sniff_image(&data).ok_or("подходят только снимки JPEG, PNG и WebP")?;
    Ok((mime.to_string(), data))
}

#[tauri::command]
fn set_cover_photo(state: State<'_, AppState>, id: String, data_url: String) -> Res<Title> {
    let (mime, data) = decode_data_url(&data_url)?;
    let mut st = state.store()?;
    let mut t = get_title(&st, &id)?;
    t.cover = Some(st.put_blob(&mime, &data).map_err(err)?);
    t.cover_source = Some("своё фото".into());
    put_title(&mut st, t)
}

// ——— Итоги и настройки ———

fn goal_of(st: &Store, year: i32) -> Goal {
    st.get::<Goal>(GOAL_KIND, &year.to_string()).ok().flatten().unwrap_or_default()
}

#[tauri::command]
fn stats(state: State<'_, AppState>, year: i32) -> Res<YearStats> {
    let st = state.store()?;
    let list = st.list::<Title>(KIND).map_err(err)?;
    Ok(stats::year_stats(&list, year, goal_of(&st, year)))
}

#[derive(Serialize)]
struct SettingsView {
    goal: Goal,
    has_poiskkino: bool,
    has_google: bool,
    key_method: &'static str,
}

#[tauri::command]
fn settings(state: State<'_, AppState>, year: i32) -> Res<SettingsView> {
    let st = state.store()?;
    let keys = state.keys();
    Ok(SettingsView {
        goal: goal_of(&st, year),
        has_poiskkino: keys.poiskkino.as_deref().is_some_and(|k| !k.is_empty()),
        has_google: keys.google.as_deref().is_some_and(|k| !k.is_empty()),
        key_method: polka_secure::method(),
    })
}

#[tauri::command]
fn set_goal(state: State<'_, AppState>, year: i32, goal: Goal) -> Res<()> {
    if !(1900..=2200).contains(&year) {
        return Err("неверный год".into());
    }
    let clean = |n: Option<u32>| n.filter(|n| (1..=1000).contains(n));
    state.store()?.put(GOAL_KIND, &year.to_string(), &Goal { books: clean(goal.books), screen: clean(goal.screen) }).map_err(err)
}

#[tauri::command]
fn set_key(state: State<'_, AppState>, name: String, value: String) -> Res<()> {
    let value = Some(value.trim().to_string()).filter(|v| !v.is_empty());
    if value.as_deref().is_some_and(|v| v.len() > 200 || v.chars().any(char::is_whitespace)) {
        return Err("ключ выглядит неверно — скопируйте его целиком, без пробелов".into());
    }
    let mut keys = state.keys.lock().map_err(err)?;
    match name.as_str() {
        "poiskkino" => keys.poiskkino = value,
        "google" => keys.google = value,
        _ => return Err("неизвестный ключ".into()),
    }
    save_keys(&state.dir, &keys)
}

// ——— Картинка года ———

/// Обложка из базы для холста картинки года: байты снимка, без адреса протокола —
/// так холст не считается «чужим» и картинку можно сохранить.
#[tauri::command]
fn cover_bytes(state: State<AppState>, id: String) -> Res<tauri::ipc::Response> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("неверный номер обложки".into());
    }
    let (_, data) = state.store()?.blob(&id).map_err(err)?.ok_or("обложки нет в базе")?;
    Ok(tauri::ipc::Response::new(data))
}

/// Имя файла без знаков, которые Windows не принимает.
#[cfg_attr(not(desktop), allow(dead_code))]
fn file_name(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c)).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let clean = if clean.is_empty() { "Полка.png".to_string() } else { clean };
    if clean.to_lowercase().ends_with(".png") {
        clean
    } else {
        format!("{clean}.png")
    }
}

/// Компьютер: картинка года — в файл через окно «Сохранить как» (по умолчанию — «Изображения»).
/// Возвращает путь; `None` — окно закрыли.
#[tauri::command]
async fn save_image(app: tauri::AppHandle, name: String, data: String) -> Res<Option<String>> {
    let bytes = B64.decode(data.as_bytes()).map_err(|_| "картинка повреждена".to_string())?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.len() > 32 << 20 {
        return Err("ожидалась картинка PNG".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut dialog = app.dialog().file().set_title("Сохранить картинку").set_file_name(file_name(&name)).add_filter("Картинка PNG", &["png"]);
        if let Ok(dir) = app.path().picture_dir() {
            if dir.is_dir() {
                dialog = dialog.set_directory(dir);
            }
        }
        dialog.save_file(move |p| {
            let _ = tx.send(p);
        });
        let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten()).await.map_err(err)?;
        let Some(p) = picked else { return Ok(None) };
        let path = p.into_path().map_err(err)?;
        std::fs::write(&path, &bytes).map_err(|e| format!("картинка не сохранилась: {e}"))?;
        Ok(Some(path.display().to_string()))
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, name, bytes);
        Err("На телефоне картинку отправляют через «Поделиться»".into())
    }
}

// ——— Протокол обложек ———

fn cover_response(state: &AppState, path: &str) -> tauri::http::Response<Vec<u8>> {
    let id = path.trim_start_matches('/');
    let found = if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        state.store().ok().and_then(|st| st.blob(id).ok().flatten())
    } else {
        None
    };
    let builder = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
    let resp = match found {
        // Номер снимка — хеш содержимого: снимок по этому адресу не меняется никогда.
        Some((mime, data)) => builder.status(200).header("Content-Type", mime).header("Cache-Control", "max-age=31536000, immutable").body(data),
        None => builder.status(404).body(Vec::new()),
    };
    resp.unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().register_uri_scheme_protocol("cover", |ctx, request| {
        let state = ctx.app_handle().state::<AppState>();
        cover_response(&state, request.uri().path())
    });
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    // Окно выбора папки для резервных копий.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_dialog::init());
    builder
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = dir.join("polka.db");
            let me = load_identity(&dir)?;
            let key = load_key(&dir, &db)?;
            let store = Store::open(&db, &key, &me.id).map_err(err)?;
            let keys = load_keys(&dir);
            backup::clean_uploads(&dir);
            app.manage(AppState { store: Arc::new(Mutex::new(store)), dir, keys: Mutex::new(keys), net: Arc::new(Net::new()), id: me.id, name: device_name(), lan: lan::Lan::default() });
            backup::start_auto(app.handle().clone());
            lan::start(app.handle().clone());
            cloud::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|_, event| {
            // Вернулись в «Полку» — пора обменяться через облако, если давно не обменивались.
            if let tauri::WindowEvent::Focused(true) = event {
                cloud::resumed();
            }
        })
        .invoke_handler(tauri::generate_handler![
            titles,
            save_title,
            delete_title,
            set_status,
            mark_episode,
            mark_season,
            search,
            from_link,
            details,
            add_title,
            set_cover_url,
            set_cover_photo,
            stats,
            settings,
            set_goal,
            set_key,
            set_tz,
            cover_bytes,
            save_image,
            backup::backup_status,
            backup::backup_password,
            backup::backup_every,
            backup::backup_dir,
            backup::backup_pick_dir,
            backup::backup_now,
            backup::backup_upload,
            backup::backup_inspect,
            backup::backup_restore,
            backup::backup_discard,
            lan::lan_status,
            lan::lan_scan,
            lan::lan_code_start,
            lan::lan_code_stop,
            lan::lan_pair,
            lan::lan_pair_qr,
            lan::lan_sync_now,
            lan::lan_unpair,
            cloud::cloud_status,
            cloud::cloud_app_passwords,
            cloud::cloud_dav_login,
            cloud::cloud_password,
            cloud::cloud_sync_now,
            cloud::cloud_every,
            cloud::cloud_disconnect,
        ])
        .run(tauri::generate_context!())
        .expect("«Полка» не запустилась");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_file_names() {
        assert_eq!(file_name("Полка — итоги 2026.png"), "Полка — итоги 2026.png");
        assert_eq!(file_name("итоги: 2026/11?"), "итоги 202611.png");
        assert_eq!(file_name("  ...  "), "Полка.png");
        assert_eq!(file_name("a\u{0}b.PNG"), "ab.PNG");
    }

    #[test]
    fn data_url() {
        let png = [0x89, b'P', b'N', b'G', 0, 1, 2, 3];
        let url = format!("data:image/png;base64,{}", B64.encode(png));
        assert_eq!(decode_data_url(&url).unwrap(), ("image/png".to_string(), png.to_vec()));
        assert!(decode_data_url("data:text/html;base64,PGh0bWw+").is_err());
        assert!(decode_data_url(&format!("data:image/png;base64,{}", B64.encode(b"%PDF"))).is_err());
        assert!(decode_data_url("нет запятой").is_err());
    }

    #[test]
    fn key_is_created_once_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("polka.db");
        let k1 = load_key(dir.path(), &db).unwrap();
        assert_eq!(load_key(dir.path(), &db).unwrap(), k1);
        // База есть, ключа нет — не создаём новый молча.
        std::fs::write(&db, b"x").unwrap();
        std::fs::remove_file(dir.path().join("key.sec")).unwrap();
        assert!(load_key(dir.path(), &db).is_err());
    }

    #[test]
    fn catalog_keys_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_keys(dir.path()).poiskkino.is_none());
        save_keys(dir.path(), &Keys { poiskkino: Some("KEY".into()), google: None }).unwrap();
        assert_eq!(load_keys(dir.path()).poiskkino.as_deref(), Some("KEY"));
    }
}

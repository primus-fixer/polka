//! Резервные копии — как в «Кармане»: пароль копии, папка, автокопии по расписанию и «Сделать копию» —
//! на компьютере; восстановление из файла — на компьютере и на телефоне.
//!
//! Телефон копии не делает: после обмена по Wi-Fi его записи попадают в копии компьютера,
//! а новый телефон получает всё обратно, когда его свяжут с компьютером.

use crate::{err, put_setting, setting, AppState, Res, APP_VERSION};
use polka_core::backup::{self, BackupKey, Checked, MadeBy, Manifest, Reader, Writer};
use polka_core::db::Store;
use polka_core::model::{Title, KIND};
use polka_core::Date;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// На устройстве (не обменивается): ключ копий, папка, как часто, последняя копия, ошибка автокопии.
pub(crate) const META_KEY: &str = "backup.key";
const META_DIR: &str = "backup.dir";
const META_EVERY: &str = "backup.every";
const META_LOCAL: &str = "backup.local";
const META_ERROR: &str = "backup.error";
/// Общая настройка: когда и где сделана последняя копия — видна и на телефоне.
pub const SETTING_LAST: &str = "backup.last";

/// Файл копии при загрузке для восстановления — не больше 4 ГБ.
const MAX_UPLOAD: u64 = 4 << 30;
const CONFLICT: &str = "Копия сделана на этом же устройстве до того, как его база началась заново, и записи в ней расходятся с нынешними. Восстановите её на чистой установке «Полки»: удалите данные приложения или поставьте его на другое устройство";

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Сбрасывает «копия делается», что бы ни случилось.
struct Running;
impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Last {
    /// Когда: секунды Unix.
    pub at: i64,
    pub path: String,
    pub auto: bool,
    pub bytes: u64,
    pub events: u64,
    pub blobs: u64,
}

#[derive(Serialize)]
pub struct Status {
    /// Копии делает это устройство (компьютер).
    pub desktop: bool,
    pub password: bool,
    pub dir: String,
    /// Папку выбрали (а не папка по умолчанию).
    pub chosen: bool,
    /// day, week или off.
    pub every: String,
    /// Сколько автокопий хранится.
    pub keep: usize,
    pub last: Option<Last>,
    /// Последняя копия по общей настройке: «секунды|устройство» — с любого устройства.
    pub shared: Option<String>,
    /// Почему не получилась последняя автокопия.
    pub error: Option<String>,
    pub running: bool,
    /// Подключено облако: оно зашифровано тем же паролем.
    pub cloud: bool,
}

fn every(st: &Store) -> String {
    match st.meta(META_EVERY).ok().flatten().as_deref() {
        Some("week") => "week".into(),
        Some("off") => "off".into(),
        _ => "day".into(),
    }
}

fn keep(every: &str) -> usize {
    if every == "week" {
        8
    } else {
        14
    }
}

/// Папка по умолчанию: «Документы/Полка — копии».
fn default_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .document_dir()
        .or_else(|_| app.path().home_dir().map(|h| h.join("Documents")))
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Полка — копии")
}

fn dir_of(app: &AppHandle, st: &Store) -> (PathBuf, bool) {
    match st.meta(META_DIR).ok().flatten().filter(|d| !d.is_empty()) {
        Some(d) => (PathBuf::from(d), true),
        None => (default_dir(app), false),
    }
}

fn status_of(app: &AppHandle, st: &Store) -> Status {
    let (dir, chosen) = dir_of(app, st);
    let every = every(st);
    Status {
        desktop: cfg!(desktop),
        password: st.meta(META_KEY).ok().flatten().is_some(),
        dir: dir.display().to_string(),
        chosen,
        keep: keep(&every),
        every,
        last: st.meta(META_LOCAL).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()),
        shared: setting(st, SETTING_LAST),
        error: st.meta(META_ERROR).ok().flatten().filter(|e| !e.is_empty()),
        running: RUNNING.load(Ordering::SeqCst),
        cloud: crate::cloud::key_meta(st).is_some(),
    }
}

#[tauri::command]
pub fn backup_status(app: AppHandle, state: State<AppState>) -> Res<Status> {
    let st = state.store()?;
    Ok(status_of(&app, &st))
}

/// Пароль копии: из него считается ключ копий (около секунды) и хранится в базе.
/// Прежние копии открываются прежним паролем.
#[tauri::command]
pub async fn backup_password(app: AppHandle, password: String) -> Res<Status> {
    if !cfg!(desktop) {
        return Err("Копии делает «Полка» на компьютере".into());
    }
    // Облако зашифровано тем же паролем: его ключ перешифровывается вместе с паролем копий.
    let cloud_key = crate::cloud::key_meta(&*app.state::<AppState>().store()?);
    let mut bytes = password.into_bytes();
    let (key, cloud_file) = tauri::async_runtime::spawn_blocking(move || {
        let made = std::str::from_utf8(&bytes).map_err(err).and_then(|p| {
            // Новый пароль — не из самых простых: им шифруются и копии, и облако.
            backup::check_new_password(p).map_err(err)?;
            let key = BackupKey::new(p).map_err(err)?;
            let cloud_file = cloud_key.as_deref().map(|ck| crate::cloud::rewrap(ck, p)).transpose()?;
            Ok((key, cloud_file))
        });
        bytes.fill(0);
        std::hint::black_box(&bytes);
        made
    })
    .await
    .map_err(err)??;
    let state = app.state::<AppState>();
    let st = state.store()?;
    st.set_meta(META_KEY, &key.to_meta()).map_err(err)?;
    if let Some(kf) = cloud_file {
        crate::cloud::save_keyfile(&st, &kf)?;
    }
    Ok(status_of(&app, &st))
}

#[tauri::command]
pub fn backup_every(app: AppHandle, state: State<AppState>, every: String) -> Res<Status> {
    if !["day", "week", "off"].contains(&every.as_str()) {
        return Err("Как часто: каждый день, раз в неделю или выключено".into());
    }
    let st = state.store()?;
    st.set_meta(META_EVERY, &every).map_err(err)?;
    Ok(status_of(&app, &st))
}

/// В папку можно писать: создаём её и пробуем записать и удалить маленький файл.
fn check_writable(dir: &Path) -> Res<()> {
    let bad = |e: std::io::Error| format!("В папку «{}» не записать: {e}", dir.display());
    std::fs::create_dir_all(dir).map_err(bad)?;
    let probe = dir.join(".polka-probe");
    std::fs::write(&probe, b"ok").map_err(bad)?;
    std::fs::remove_file(&probe).map_err(bad)
}

/// Папка копий: путь или `None` — папка по умолчанию.
#[tauri::command]
pub fn backup_dir(app: AppHandle, state: State<AppState>, path: Option<String>) -> Res<Status> {
    let st = state.store()?;
    match path.filter(|p| !p.trim().is_empty()) {
        Some(p) => {
            let dir = PathBuf::from(p.trim());
            if !dir.is_absolute() {
                return Err("Нужен полный путь к папке".into());
            }
            check_writable(&dir)?;
            st.set_meta(META_DIR, &dir.display().to_string()).map_err(err)?;
        }
        None => st.set_meta(META_DIR, "").map_err(err)?,
    }
    Ok(status_of(&app, &st))
}

/// Папка копий в окне системы.
#[tauri::command]
pub async fn backup_pick_dir(app: AppHandle) -> Res<Status> {
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let current = {
            let state = app.state::<AppState>();
            let st = state.store()?;
            dir_of(&app, &st).0
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut dialog = app.dialog().file().set_title("Папка для резервных копий").set_can_create_directories(true);
        if current.is_dir() {
            dialog = dialog.set_directory(&current);
        }
        dialog.pick_folder(move |p| {
            let _ = tx.send(p);
        });
        let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten()).await.map_err(err)?;
        if let Some(p) = picked {
            let dir = p.into_path().map_err(err)?;
            check_writable(&dir)?;
            let state = app.state::<AppState>();
            state.store()?.set_meta(META_DIR, &dir.display().to_string()).map_err(err)?;
        }
    }
    let state = app.state::<AppState>();
    let st = state.store()?;
    Ok(status_of(&app, &st))
}

/// Копия сейчас, в папку копий.
#[tauri::command]
pub async fn backup_now(app: AppHandle) -> Res<Last> {
    tauri::async_runtime::spawn_blocking(move || make(&app, false)).await.map_err(err)?
}

/// «2026-11-14 1830» по местному времени (смещение присылает экран).
fn stamp(at: i64, tz_minutes: i64) -> String {
    let local = at + tz_minutes * 60;
    let day = Date::from_epoch_days(local.div_euclid(86_400) as i32);
    let sec = local.rem_euclid(86_400);
    format!("{day} {:02}{:02}", sec / 3600, sec % 3600 / 60)
}

/// Имя без повторов: «… (2).polka».
fn unique(dir: &Path, base: &str) -> PathBuf {
    let mut path = dir.join(format!("{base}.{}", backup::EXT));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).{}", backup::EXT));
        n += 1;
    }
    path
}

/// Делает копию: события под замком базы сразу, обложки — по одной, чтобы экраны не ждали.
/// Пишется во временный файл, читается обратно целиком и только потом получает своё имя.
fn make(app: &AppHandle, auto: bool) -> Res<Last> {
    if !cfg!(desktop) {
        return Err("Копии делает «Полка» на компьютере".into());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Копия уже делается — подождите немного".into());
    }
    let _running = Running;
    let state = app.state::<AppState>();
    let at = polka_core::now_secs();
    let (key, dir, events, blobs, manifest, tz) = {
        let st = state.store()?;
        let key = st.meta(META_KEY).map_err(err)?.ok_or_else(|| "Сначала задайте пароль копии".to_string())?;
        let key = BackupKey::from_meta(&key).map_err(err)?;
        let events = st.events_missing(&Default::default()).map_err(err)?;
        let blobs = st.present_blobs().map_err(err)?;
        let by = MadeBy { app: APP_VERSION.into(), created: at, device: state.name.clone(), device_id: st.device().to_string() };
        let titles = st.list::<Title>(KIND).map_err(err)?.len() as u64;
        let manifest = Manifest::new(&by, events.len() as u64, blobs.len() as u64, titles);
        (key, dir_of(app, &st).0, events, blobs, manifest, crate::tz_minutes(&st))
    };
    check_writable(&dir)?;
    let path = unique(&dir, &format!("Полка {} {}", if auto { "автокопия" } else { "копия" }, stamp(at, tz)));
    let tmp = path.with_file_name(format!("{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("копия")));
    let written = (|| -> Res<backup::Made> {
        let file = File::create(&tmp).map_err(err)?;
        let mut w = Writer::new(BufWriter::with_capacity(1 << 20, file), &key, &manifest).map_err(err)?;
        w.events(&events).map_err(err)?;
        for id in &blobs {
            let blob = state.store()?.blob(id).map_err(err)?;
            if let Some((mime, data)) = blob {
                w.blob(id, &mime, &data).map_err(err)?;
            }
        }
        let (made, out) = w.finish().map_err(err)?;
        out.into_inner().map_err(|e| err(e.error()))?.sync_all().map_err(err)?;
        // Проверка: копия читается целиком сохранённым ключом и сходится по числу записей.
        let r = Reader::open_with_key(BufReader::new(File::open(&tmp).map_err(err)?), &key).map_err(err)?;
        let checked = backup::verify(r, "", |_| Ok(false)).map_err(err)?;
        if (checked.events, checked.blobs) != (made.events, made.blobs) {
            return Err("копия не сошлась при проверке".into());
        }
        std::fs::rename(&tmp, &path).map_err(err)?;
        Ok(made)
    })();
    let made = match written {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Копия не сделана: {e}"));
        }
    };
    let last = Last { at, path: path.display().to_string(), auto, bytes: made.bytes, events: made.events, blobs: made.blobs };
    {
        let mut st = state.store()?;
        st.set_meta(META_LOCAL, &serde_json::to_string(&last).map_err(err)?).map_err(err)?;
        st.set_meta(META_ERROR, "").map_err(err)?;
        // Когда и где была копия — общая настройка: её увидит и телефон.
        put_setting(&mut st, SETTING_LAST, &format!("{at}|{}", state.name))?;
    }
    if auto {
        let every = state.store().map(|st| every(&st)).unwrap_or_else(|_| "day".into());
        prune(&dir, keep(&every));
    }
    let _ = app.emit("backup", &last);
    Ok(last)
}

/// Оставляет последние автокопии. Трогает только свои файлы: по имени и по началу файла.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut autos: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("Полка автокопия ") && name.ends_with(&format!(".{}", backup::EXT)) && backup::is_backup(p)
        })
        .collect();
    // Имена со временем «ГГГГ-ММ-ДД ЧЧММ» сортируются по времени.
    autos.sort();
    while autos.len() > keep {
        let _ = std::fs::remove_file(autos.remove(0));
    }
}

/// Пора делать автокопию: задан пароль, есть записи, прошло больше суток (или недели) с прошлой копии.
fn due(st: &Store) -> Res<bool> {
    let hours = match every(st).as_str() {
        "day" => 20,
        "week" => 6 * 24 + 12,
        _ => return Ok(false),
    };
    if st.meta(META_KEY).map_err(err)?.is_none() || st.list::<Title>(KIND).map_err(err)?.is_empty() {
        return Ok(false);
    }
    let last = st.meta(META_LOCAL).map_err(err)?.and_then(|s| serde_json::from_str::<Last>(&s).ok()).map(|l| l.at).unwrap_or(0);
    Ok(polka_core::now_secs() - last >= hours * 3600)
}

/// Автокопии на компьютере: через полминуты после запуска и потом каждые четверть часа проверяем, не пора ли.
pub fn start_auto(app: AppHandle) {
    if !cfg!(desktop) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        loop {
            let state = app.state::<AppState>();
            let is_due = state.store().ok().map(|st| due(&st).unwrap_or(false)).unwrap_or(false);
            if is_due {
                if let Err(e) = make(&app, true) {
                    if let Ok(st) = state.store() {
                        let _ = st.set_meta(META_ERROR, &e);
                    }
                    let _ = app.emit("backup", ());
                }
            }
            std::thread::sleep(Duration::from_secs(15 * 60));
        }
    });
}

// ——— Восстановление ———

fn uploads(state: &AppState) -> PathBuf {
    state.dir.join("restore")
}

/// Загрузки, брошенные на полпути, убираем при запуске.
pub fn clean_uploads(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.join("restore"));
}

pub(crate) fn upload_path(state: &AppState, id: &str) -> Res<PathBuf> {
    if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("неверный номер загрузки".into());
    }
    Ok(uploads(state).join(format!("{id}.part")))
}

#[derive(Serialize)]
pub struct Upload {
    pub id: String,
    pub size: u64,
}

/// Файл (копия или книга) приходит с экрана кусками (на телефоне его иначе не прочитать):
/// заголовки `x-upload` (номер или `new`) и `x-offset`.
#[tauri::command]
pub fn backup_upload(state: State<AppState>, request: tauri::ipc::Request<'_>) -> Res<Upload> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("ожидались байты файла".into());
    };
    let head = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string).ok_or_else(|| format!("нет заголовка {name}"));
    let offset: u64 = head("x-offset")?.parse().map_err(|_| "неверное смещение".to_string())?;
    let given = head("x-upload")?;
    let id = if given == "new" { polka_core::new_id().map_err(err)? } else { given };
    let path = upload_path(&state, &id)?;
    std::fs::create_dir_all(uploads(&state)).map_err(err)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if offset != size {
        return Err("Файл пришёл не по порядку — выберите его снова".into());
    }
    if size + data.len() as u64 > MAX_UPLOAD {
        let _ = std::fs::remove_file(&path);
        return Err("Файл слишком большой для копии «Полки»".into());
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(err)?;
    f.write_all(data).map_err(err)?;
    Ok(Upload { id, size: size + data.len() as u64 })
}

/// Проверка копии целиком; события этого же устройства сверяются с базой.
fn check_file(state: &AppState, path: &Path, password: &str) -> Res<Checked> {
    let device = state.store()?.device().to_string();
    let r = Reader::open(BufReader::new(File::open(path).map_err(err)?), password).map_err(err)?;
    let checked = backup::verify(r, &device, |ev| {
        let st = state.store.lock().map_err(|_| polka_core::Error::Backup("база занята".into()))?;
        backup::conflicts_with(&st, ev)
    })
    .map_err(err)?;
    if checked.conflicts > 0 {
        return Err(CONFLICT.into());
    }
    Ok(checked)
}

#[derive(Serialize)]
pub struct Info {
    pub created: i64,
    pub device: String,
    pub app: String,
    pub titles: u64,
    pub events: u64,
    pub blobs: u64,
    pub bytes: u64,
}

/// Что в копии: проверяет пароль и файл целиком, базу не меняет.
#[tauri::command]
pub async fn backup_inspect(app: AppHandle, id: String, password: String) -> Res<Info> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = upload_path(&state, &id)?;
        let bytes = std::fs::metadata(&path).map_err(|_| "Файл копии не загружен — выберите его снова".to_string())?.len();
        let c = check_file(&state, &path, &password)?;
        let m = c.manifest;
        Ok(Info { created: m.created, device: m.device, app: m.app, titles: m.titles, events: c.events, blobs: c.blobs, bytes })
    })
    .await
    .map_err(err)?
}

#[derive(Serialize)]
pub struct Restored {
    pub events: u64,
    pub blobs: u64,
    /// Сколько записей прибавилось на полке.
    pub titles: u64,
    /// Полка после восстановления — экран обновляется сразу.
    pub list: Vec<Title>,
}

/// Добавляет копию к записям: сначала проверяет файл целиком, потом применяет —
/// как обмен: повторы пропускаются, более позднее изменение побеждает.
#[tauri::command]
pub async fn backup_restore(app: AppHandle, id: String, password: String) -> Res<Restored> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = upload_path(&state, &id)?;
        check_file(&state, &path, &password)?;
        let r = Reader::open(BufReader::new(File::open(&path).map_err(err)?), &password).map_err(err)?;
        let mut st = state.store()?;
        let before = st.list::<Title>(KIND).map_err(err)?.len();
        let added = backup::restore_into(r, &mut st).map_err(err)?;
        let list = st.list::<Title>(KIND).map_err(err)?;
        drop(st);
        let _ = std::fs::remove_file(&path);
        Ok(Restored { events: added.events, blobs: added.blobs, titles: list.len().saturating_sub(before) as u64, list })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn backup_discard(state: State<AppState>, id: String) -> Res<()> {
    let path = upload_path(&state, &id)?;
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_the_latest_autocopies_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let real = |name: &str| std::fs::write(dir.path().join(name), [backup::MAGIC.as_slice(), b"rest"].concat()).unwrap();
        for d in 1..=5 {
            real(&format!("Полка автокопия 2026-11-0{d} 1000.polka"));
        }
        real("Полка копия 2026-11-01 0900.polka");
        // Похожее имя, но не копия «Полки» — не трогаем.
        std::fs::write(dir.path().join("Полка автокопия 2026-10-01 1000.polka"), b"my notes").unwrap();
        std::fs::write(dir.path().join("список.txt"), b"x").unwrap();
        prune(dir.path(), 3);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(
            left,
            [
                "Полка автокопия 2026-10-01 1000.polka",
                "Полка автокопия 2026-11-03 1000.polka",
                "Полка автокопия 2026-11-04 1000.polka",
                "Полка автокопия 2026-11-05 1000.polka",
                "Полка копия 2026-11-01 0900.polka",
                "список.txt",
            ]
        );
    }

    #[test]
    fn stamps_and_unique_names() {
        // 14.11.2026 13:20 по UTC — это 18:20 в Алматы (UTC+5).
        assert_eq!(stamp(1_794_662_400, 300), "2026-11-14 1820");
        assert_eq!(stamp(1_794_662_400, 0), "2026-11-14 1320");
        let dir = tempfile::tempdir().unwrap();
        let a = unique(dir.path(), "Полка копия 2026-11-14 1820");
        std::fs::write(&a, b"x").unwrap();
        let b = unique(dir.path(), "Полка копия 2026-11-14 1820");
        assert!(b.to_string_lossy().ends_with("Полка копия 2026-11-14 1820 (2).polka"));
    }
}

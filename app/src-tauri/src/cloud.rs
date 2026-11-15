//! Облако: Яндекс Диск по логину и паролю приложения или свой сервер WebDAV. Пароль копий
//! и обмен в фоне. Устроено как в «Кармане»: тот же код, своя папка «Полка».
//!
//! Пароль приложения Яндекса (или пароль своего сервера) хранится только на этом устройстве:
//! в зашифрованной базе и ещё под защитой системы. В копии и на другие устройства он не уходит.
//!
//! Обмен идёт сам: вскоре после запуска, через минуту после изменений, каждые 15 минут
//! и при возвращении в «Полку», если с прошлого обмена прошло больше двух минут.
//! Настройка «Только по кнопке» оставляет одну кнопку «Обменяться сейчас».
//!
//! Пароль облака — это пароль копий: на компьютере они совпадают, смена пароля копий
//! перешифровывает ключ облака. Телефон пароль копий не хранит — его вводят один раз при подключении.

use crate::{err, AppState, Res};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use polka_cloud::dav::{self, Dav};
use polka_core::backup::{check_password, BackupKey, Kdf};
use polka_core::cloud::{self as pc, CloudKey, KeyFile, Report, Round, State as CloudState, Step};
use polka_core::db::Store;
use polka_core::error::{CloudFail, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// На устройстве (не синхронизируется): yadisk или webdav.
const META_PROVIDER: &str = "cloud.provider";
/// Адрес сервера WebDAV и логин.
const META_DAV: &str = "cloud.dav";
/// Пароль приложения или сервера — под защитой системы.
const META_PASSWORD: &str = "cloud.password";
const META_KEY: &str = "cloud.key";
const META_STATE: &str = "cloud.state";
const META_EVERY: &str = "cloud.every";
const META_LAST: &str = "cloud.last";
const META_ERROR: &str = "cloud.error";
/// Ключ облака под новым паролем копий: уйдёт в облако при ближайшем обмене.
const META_KEYFILE: &str = "cloud.keyfile";
/// В облаке уже есть данные «Полки»: «1» или «0».
const META_EXISTS: &str = "cloud.exists";

const EVERY: Duration = Duration::from_secs(15 * 60);
const RESUME_GAP: Duration = Duration::from_secs(2 * 60);
const RETRY: Duration = Duration::from_secs(5 * 60);

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static MANUAL: AtomicBool = AtomicBool::new(false);
static RESUMED: AtomicBool = AtomicBool::new(false);
static PROGRESS: Mutex<Option<Step>> = Mutex::new(None);
/// Один обмен за раз; отключение ждёт, пока идущий обмен остановится.
static ROUND: Mutex<()> = Mutex::new(());

struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

static WAKE: Wake = Wake { flag: Mutex::new(false), cv: Condvar::new() };

fn wake() {
    if let Ok(mut f) = WAKE.flag.lock() {
        *f = true;
    }
    WAKE.cv.notify_all();
}

fn sleep_or_wake(d: Duration) {
    let Ok(g) = WAKE.flag.lock() else {
        std::thread::sleep(d);
        return;
    };
    if let Ok((mut g, _)) = WAKE.cv.wait_timeout_while(g, d, |woken| !*woken) {
        *g = false;
    }
}

/// Обменяться при первой возможности (кнопка, подключение, новый пароль).
pub fn sync_now() {
    MANUAL.store(true, Ordering::SeqCst);
    wake();
}

/// «Полка» снова на экране.
pub fn resumed() {
    RESUMED.store(true, Ordering::SeqCst);
    wake();
}

fn now() -> i64 {
    polka_cloud::now()
}

fn meta(st: &Store, k: &str) -> Option<String> {
    st.meta(k).ok().flatten().filter(|v| !v.is_empty())
}

fn clear(st: &Store, keys: &[&str]) {
    for k in keys {
        let _ = st.set_meta(k, "");
    }
}

// ——— Как подключено облако ———

/// Способ подключения.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    /// Яндекс Диск по логину и паролю приложения (WebDAV).
    YaDisk,
    /// Другой сервер WebDAV: Nextcloud, NAS.
    WebDav,
}

impl Link {
    fn code(self) -> &'static str {
        match self {
            Link::YaDisk => "yadisk",
            Link::WebDav => "webdav",
        }
    }

    fn parse(s: &str) -> Option<Link> {
        match s {
            "yadisk" => Some(Link::YaDisk),
            "webdav" => Some(Link::WebDav),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Link::YaDisk => "Яндекс Диск",
            Link::WebDav => "Сервер WebDAV",
        }
    }
}

/// Сервер WebDAV и логин (пароль хранится отдельно).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DavLogin {
    url: String,
    user: String,
}

fn link_of(st: &Store) -> Option<Link> {
    meta(st, META_PROVIDER).and_then(|p| Link::parse(&p))
}

fn dav_of(st: &Store) -> Option<DavLogin> {
    meta(st, META_DAV).and_then(|t| serde_json::from_str(&t).ok())
}

/// Пароль из базы: он лежит там под защитой системы.
fn dav_password(st: &Store) -> Res<Option<String>> {
    let Some(text) = meta(st, META_PASSWORD) else { return Ok(None) };
    let blob = B64.decode(text.trim()).map_err(err)?;
    let plain = polka_secure::unprotect(&blob).map_err(|e| format!("пароль облака не достать из хранилища системы: {e}"))?;
    String::from_utf8(plain).map(Some).map_err(err)
}

/// Для сквозных проверок в отладочной сборке: имитация облака на этом компьютере.
#[cfg(debug_assertions)]
fn mock_base() -> Option<String> {
    std::env::var("POLKA_CLOUD_MOCK").ok().filter(|b| b.starts_with("http://127.0.0.1:"))
}

/// Адрес Яндекс Диска по WebDAV; в сквозных проверках — имитация.
fn yadisk_url() -> String {
    #[cfg(debug_assertions)]
    if let Some(base) = mock_base() {
        return format!("{base}/dav");
    }
    dav::YANDEX.into()
}

/// Подключение по входу этого устройства.
fn open(st: &Store) -> Res<Dav> {
    link_of(st).ok_or("Сначала войдите в облако")?;
    let login = dav_of(st).ok_or("Сначала войдите в облако")?;
    let password = dav_password(st)?.ok_or("Сначала войдите в облако")?;
    Ok(Dav::new(&login.url, &login.user, &password))
}

/// Страницу открываем, только если она у Яндекса.
fn safe_page(url: &str) -> bool {
    const PAGES: [&str; 2] = ["https://id.yandex.ru/", "https://yandex.ru/support/"];
    PAGES.iter().any(|p| url.starts_with(p)) && !url.chars().any(|c| c.is_whitespace() || c == '"')
}

fn open_page(app: &AppHandle, url: &str) {
    if !safe_page(url) {
        return;
    }
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        eprintln!("страница не открылась: {e}");
    }
}

// ——— Состояние для экрана ———

/// Вход по паролю: сервер и логин (пароль экран не получает).
#[derive(Serialize)]
pub struct DavView {
    url: String,
    host: String,
    user: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Last {
    pub at: i64,
    pub received: usize,
    pub sent: usize,
    pub files_received: usize,
    pub files_sent: usize,
    pub files_waiting: usize,
    /// Файлы книг: пришло, ушло, ещё не в облаке целиком.
    #[serde(default)]
    pub books_received: usize,
    #[serde(default)]
    pub books_sent: usize,
    #[serde(default)]
    pub books_waiting: usize,
    #[serde(default)]
    pub problems: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Problem {
    pub kind: CloudFail,
    pub text: String,
    pub at: i64,
}

#[derive(Serialize)]
pub struct Status {
    desktop: bool,
    /// off — не подключено; password — нужен пароль копий; relogin — пароль приложения не подходит; ready — подключено.
    state: &'static str,
    provider: Option<&'static str>,
    title: Option<&'static str>,
    dav: Option<DavView>,
    /// В облаке уже есть данные «Полки»: пароль вводят, а не придумывают.
    cloud_exists: bool,
    /// На этом устройстве задан пароль копий.
    backup_password: bool,
    /// auto — обмен сам, manual — только по кнопке.
    every: String,
    running: bool,
    progress: Option<Step>,
    last: Option<Last>,
    error: Option<Problem>,
    /// Новый пароль копий ещё не ушёл в облако.
    key_pending: bool,
    /// Сообщение после действия.
    notice: Option<String>,
}

fn every(st: &Store) -> String {
    if meta(st, META_EVERY).as_deref() == Some("manual") {
        "manual".into()
    } else {
        "auto".into()
    }
}

fn status_of(st: &Store, notice: Option<String>) -> Status {
    let provider = link_of(st);
    let (password, key) = (meta(st, META_PASSWORD).is_some(), meta(st, META_KEY).is_some());
    let name = match (provider.is_some(), password, key) {
        (false, ..) => "off",
        (true, true, true) => "ready",
        (true, true, false) => "password",
        (true, false, true) => "relogin",
        (true, false, false) => "off",
    };
    Status {
        desktop: cfg!(desktop),
        state: name,
        provider: provider.map(Link::code),
        title: provider.map(Link::title),
        dav: provider.and(dav_of(st)).map(|l| DavView { host: dav::host_of(&l.url).to_string(), url: l.url, user: l.user }),
        cloud_exists: meta(st, META_EXISTS).as_deref() == Some("1"),
        backup_password: meta(st, crate::backup::META_KEY).is_some(),
        every: every(st),
        running: RUNNING.load(Ordering::SeqCst),
        progress: PROGRESS.lock().ok().and_then(|p| *p),
        last: meta(st, META_LAST).and_then(|s| serde_json::from_str(&s).ok()),
        error: meta(st, META_ERROR).and_then(|s| serde_json::from_str(&s).ok()),
        key_pending: meta(st, META_KEYFILE).is_some(),
        notice,
    }
}

fn status(app: &AppHandle, notice: Option<String>) -> Res<Status> {
    let state = app.state::<AppState>();
    let st = state.store()?;
    Ok(status_of(&st, notice))
}

// ——— Команды ———

#[tauri::command]
pub fn cloud_status(app: AppHandle) -> Res<Status> {
    status(&app, None)
}

/// Страница Яндекс ID, где создают пароль приложения.
#[tauri::command]
pub fn cloud_app_passwords(app: AppHandle) -> Res<()> {
    open_page(&app, "https://id.yandex.ru/security");
    Ok(())
}

/// Вход по логину и паролю: Яндекс Диск (пароль приложения) или свой сервер WebDAV.
/// Проверяет вход, создаёт папку «Полка» и узнаёт, есть ли в ней данные.
#[tauri::command]
pub async fn cloud_dav_login(app: AppHandle, kind: String, url: Option<String>, user: String, password: String) -> Res<Status> {
    let link = Link::parse(&kind).ok_or("Неизвестное облако")?;
    let mut bytes = password.into_bytes();
    tauri::async_runtime::spawn_blocking(move || {
        let r = std::str::from_utf8(&bytes).map_err(err).and_then(|p| dav_login(&app, link, url.as_deref(), &user, p));
        bytes.fill(0);
        std::hint::black_box(&bytes);
        r
    })
    .await
    .map_err(err)?
}

fn dav_login(app: &AppHandle, link: Link, url: Option<&str>, user: &str, password: &str) -> Res<Status> {
    let user = user.trim();
    // Пароль приложения Яндекса — 16 латинских букв; пробелы при вставке убираем.
    let password: String = if link == Link::YaDisk { password.chars().filter(|c| !c.is_whitespace()).collect() } else { password.to_string() };
    if user.is_empty() {
        return Err(if link == Link::YaDisk { "Введите логин Яндекса" } else { "Введите логин" }.into());
    }
    if password.is_empty() {
        return Err(if link == Link::YaDisk { "Введите пароль приложения" } else { "Введите пароль" }.into());
    }
    let url = match link {
        Link::YaDisk => yadisk_url(),
        Link::WebDav => dav::server_url(url.unwrap_or(""), cfg!(debug_assertions))?,
    };
    let exists = Dav::new(&url, user, &password).connect().map_err(err)?;
    let blob = polka_secure::protect(password.as_bytes()).map_err(|e| format!("пароль не сохранить в хранилище системы: {e}"))?;
    let login = DavLogin { url, user: user.to_string() };
    let state = app.state::<AppState>();
    let st = state.store()?;
    // То же облако, что было (пароль сменили): обмен продолжается без пароля копий.
    let same = link_of(&st) == Some(link) && dav_of(&st).as_ref() == Some(&login) && meta(&st, META_KEY).is_some();
    if !same {
        clear(&st, &[META_KEY, META_STATE, META_LAST, META_KEYFILE]);
    }
    st.set_meta(META_PROVIDER, link.code()).map_err(err)?;
    st.set_meta(META_DAV, &serde_json::to_string(&login).map_err(err)?).map_err(err)?;
    st.set_meta(META_PASSWORD, &B64.encode(blob)).map_err(err)?;
    st.set_meta(META_EXISTS, if exists || same { "1" } else { "0" }).map_err(err)?;
    st.set_meta(META_ERROR, "").map_err(err)?;
    let s = status_of(&st, same.then(|| "Вход обновлён — обмен продолжается".to_string()));
    drop(st);
    if same {
        sync_now();
    }
    Ok(s)
}

/// Пароль копий: открывает ключ облака, которое уже есть, или создаёт новое.
#[tauri::command]
pub async fn cloud_password(app: AppHandle, password: String) -> Res<Status> {
    let mut bytes = password.into_bytes();
    tauri::async_runtime::spawn_blocking(move || {
        let r = std::str::from_utf8(&bytes).map_err(err).and_then(|p| set_password(&app, p));
        bytes.fill(0);
        std::hint::black_box(&bytes);
        r
    })
    .await
    .map_err(err)?
}

fn set_password(app: &AppHandle, password: &str) -> Res<Status> {
    check_password(password).map_err(err)?;
    let state = app.state::<AppState>();
    let (conn, backup_meta) = {
        let st = state.store()?;
        (open(&st)?, meta(&st, crate::backup::META_KEY))
    };
    let existing = pc::read_key_file(&conn).map_err(err)?;
    // Новое облако — новый пароль на Диске: файлы там может скачать любой, кто войдёт в аккаунт,
    // поэтому самые простые пароли не годятся. Пароль облака, которое уже есть, не проверяем.
    if existing.is_none() {
        if let Err(e) = polka_core::backup::check_new_password(password) {
            let hint = if cfg!(desktop) && backup_meta.is_some() { " Смените пароль копий в разделе «Резервная копия» и подключите облако снова." } else { "" };
            return Err(format!("{e}{hint}"));
        }
    }
    // На компьютере пароль облака и пароль копий — один и тот же.
    let mut set_backup = false;
    let mut notice = None;
    if cfg!(desktop) {
        match backup_meta.as_deref().map(BackupKey::from_meta).transpose().map_err(err)? {
            Some(bk) if bk.matches(password).map_err(err)? => {}
            Some(_) if existing.is_none() => {
                return Err("Это не пароль копий этого компьютера. Введите пароль, которым зашифрованы резервные копии, — сменить его можно в разделе «Резервная копия»".into());
            }
            Some(_) => {
                set_backup = true;
                notice = Some("Пароль копий на этом компьютере теперь тот же, что у облака. Новые копии делаются с ним, прежние открываются прежним паролем.".into());
            }
            None => {
                set_backup = true;
                notice = Some("Этот пароль теперь и пароль резервных копий на этом компьютере.".into());
            }
        }
    }
    let (key, _created) = pc::join(&conn, password, Kdf::DEFAULT, now()).map_err(err)?;
    let backup_key = if set_backup { Some(BackupKey::new(password).map_err(err)?) } else { None };
    let st = state.store()?;
    st.set_meta(META_KEY, &key.to_meta()).map_err(err)?;
    clear(&st, &[META_STATE, META_ERROR, META_KEYFILE]);
    st.set_meta(META_EXISTS, "1").map_err(err)?;
    if let Some(bk) = backup_key {
        st.set_meta(crate::backup::META_KEY, &bk.to_meta()).map_err(err)?;
    }
    let s = status_of(&st, notice);
    drop(st);
    // Раздел копий показывает, задан ли пароль и зашифровано ли им облако.
    let _ = app.emit("backup", ());
    sync_now();
    Ok(s)
}

#[tauri::command]
pub fn cloud_sync_now(app: AppHandle) -> Res<Status> {
    let state = app.state::<AppState>();
    let st = state.store()?;
    if !ready(&st) {
        return Err("Облако не подключено".into());
    }
    let s = status_of(&st, None);
    drop(st);
    sync_now();
    Ok(s)
}

#[tauri::command]
pub fn cloud_every(app: AppHandle, every: String) -> Res<Status> {
    if !["auto", "manual"].contains(&every.as_str()) {
        return Err("Обмен: сам или только по кнопке".into());
    }
    {
        let state = app.state::<AppState>();
        state.store()?.set_meta(META_EVERY, &every).map_err(err)?;
    }
    status(&app, None)
}

/// Отключает облако на этом устройстве; `wipe` — ещё и удаляет данные «Полки» из облака.
#[tauri::command]
pub async fn cloud_disconnect(app: AppHandle, wipe: bool) -> Res<Status> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        CANCEL.store(true, Ordering::SeqCst);
        // Ждём, пока идущий обмен остановится (он проверяет остановку между запросами).
        let _round = ROUND.lock().unwrap_or_else(|e| e.into_inner());
        let (loaded, yadisk) = {
            let st = state.store()?;
            (open(&st), link_of(&st) == Some(Link::YaDisk))
        };
        let done = (|| -> Res<()> {
            let Ok(conn) = loaded else {
                return if wipe { Err("Чтобы удалить данные из облака, сначала войдите в него".into()) } else { Ok(()) };
            };
            if wipe {
                pc::wipe(&conn, &AtomicBool::new(false)).map_err(err)?;
                // Пустые папки «Полки» убираем; не вышло — не страшно.
                let _ = conn.tidy();
            }
            Ok(())
        })();
        CANCEL.store(false, Ordering::SeqCst);
        // Данные из облака не удалились — остаёмся подключёнными.
        if wipe {
            done?;
        }
        let st = state.store()?;
        clear(&st, &[META_PROVIDER, META_PASSWORD, META_DAV, META_KEY, META_STATE, META_LAST, META_ERROR, META_KEYFILE, META_EXISTS]);
        let mut note = if wipe { "Облако отключено, данные «Полки» из него удалены." } else { "Облако отключено. Данные «Полки» в нём остались — его можно подключить снова." }.to_string();
        if yadisk {
            note.push_str(" Пароль приложения больше не нужен «Полке» — его можно удалить в Яндекс ID.");
        }
        let s = status_of(&st, Some(note));
        drop(st);
        let _ = app.emit("backup", ());
        Ok(s)
    })
    .await
    .map_err(err)?
}

/// Ключ облака на этом устройстве, если облако подключено.
pub fn key_meta(st: &Store) -> Option<String> {
    meta(st, META_KEY)
}

/// Пароль копий сменили: тот же ключ облака под новым паролем (около секунды).
pub fn rewrap(key_meta: &str, password: &str) -> Res<KeyFile> {
    let key = CloudKey::from_meta(key_meta).map_err(err)?;
    key.key_file(password, Kdf::DEFAULT, now()).map_err(err)
}

/// Новый ключ облака уйдёт в облако при ближайшем обмене.
pub fn save_keyfile(st: &Store, kf: &KeyFile) -> Res<()> {
    st.set_meta(META_KEYFILE, &String::from_utf8_lossy(&kf.to_bytes())).map_err(err)?;
    sync_now();
    Ok(())
}

// ——— Обмен в фоне ———

struct Loaded {
    conn: Res<Dav>,
    key: CloudKey,
    state: CloudState,
    keyfile: Option<String>,
}

fn load(st: &Store) -> Option<Loaded> {
    if !ready(st) {
        return None;
    }
    Some(Loaded {
        conn: open(st),
        key: CloudKey::from_meta(&meta(st, META_KEY)?).ok()?,
        state: meta(st, META_STATE).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
        keyfile: meta(st, META_KEYFILE),
    })
}

/// Событие для экрана: обмен идёт или закончился; сколько пришло нового.
#[derive(Clone, Serialize)]
struct CloudEvent {
    running: bool,
    received: usize,
    step: Option<Step>,
}

fn emit(app: &AppHandle, ev: CloudEvent) {
    let _ = app.emit("cloud", ev);
}

struct RoundDone;

impl Drop for RoundDone {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
        if let Ok(mut p) = PROGRESS.lock() {
            *p = None;
        }
    }
}

/// Один обмен. Возвращает, что пошло не так.
fn run_round(app: &AppHandle) -> Option<CloudFail> {
    let Ok(_round) = ROUND.try_lock() else { return None };
    let state = app.state::<AppState>();
    let c = state.store.lock().ok().and_then(|st| load(&st))?;
    RUNNING.store(true, Ordering::SeqCst);
    let _done = RoundDone;
    emit(app, CloudEvent { running: true, received: 0, step: None });
    let conn = match c.conn {
        Ok(s) => s,
        Err(e) => return fail(app, &state, CloudFail::Auth, e),
    };
    let mut cstate = c.state;
    let mut last_emit = Instant::now();
    let mut result: polka_core::Result<Report> = Ok(Report::default());
    if let Some(kf_text) = &c.keyfile {
        match KeyFile::parse(kf_text.as_bytes()).and_then(|kf| pc::put_key_file(&conn, &c.key, &kf)) {
            Ok(()) => {
                if let Ok(st) = state.store.lock() {
                    // Пока шёл обмен, пароль могли сменить ещё раз — тогда ждёт уже новый ключ.
                    if meta(&st, META_KEYFILE).as_deref() == Some(kf_text) {
                        clear(&st, &[META_KEYFILE]);
                    }
                }
            }
            Err(e) => result = Err(e),
        }
    }
    if result.is_ok() {
        let mut progress = |step: Step| {
            if let Ok(mut p) = PROGRESS.lock() {
                *p = Some(step);
            }
            if last_emit.elapsed() >= Duration::from_millis(400) {
                last_emit = Instant::now();
                emit(app, CloudEvent { running: true, received: 0, step: Some(step) });
            }
        };
        result = pc::sync(&Round { cloud: &conn, store: &state.store, key: &c.key, now: now(), cancel: &CANCEL }, &mut cstate, &mut progress);
    }
    let Ok(st) = state.store.lock() else { return None };
    let _ = st.set_meta(META_STATE, &serde_json::to_string(&cstate).unwrap_or_default());
    match result {
        Ok(r) => {
            let last = Last { at: now(), received: r.received, sent: r.sent, files_received: r.files_received, files_sent: r.files_sent, files_waiting: r.files_waiting, books_received: r.books_received, books_sent: r.books_sent, books_waiting: r.books_waiting, problems: r.problems };
            let _ = st.set_meta(META_LAST, &serde_json::to_string(&last).unwrap_or_default());
            clear(&st, &[META_ERROR]);
            drop(st);
            emit(app, CloudEvent { running: false, received: r.received + r.files_received + r.books_received, step: None });
            None
        }
        Err(e) => {
            drop(st);
            let (kind, text) = match e {
                Error::Cloud(k, t) => (k, t),
                Error::CloudPassword => (CloudFail::Replaced, e.to_string()),
                other => (CloudFail::Server, format!("Обмен с облаком не получился: {other}")),
            };
            if kind == CloudFail::Cancelled {
                emit(app, CloudEvent { running: false, received: 0, step: None });
                return None;
            }
            fail(app, &state, kind, text)
        }
    }
}

/// Запоминает, что пошло не так, и что для этого нужно сделать.
fn fail(app: &AppHandle, state: &AppState, kind: CloudFail, text: String) -> Option<CloudFail> {
    if let Ok(st) = state.store.lock() {
        match kind {
            // Пароль приложения не подходит: войти снова — ключ облака остаётся, пароль копий не понадобится.
            CloudFail::Auth => clear(&st, &[META_PASSWORD]),
            // Данных в облаке нет или облако создано заново: нужен пароль копий.
            CloudFail::Gone => {
                clear(&st, &[META_KEY, META_STATE, META_KEYFILE]);
                let _ = st.set_meta(META_EXISTS, "0");
            }
            CloudFail::Replaced => {
                clear(&st, &[META_KEY, META_STATE, META_KEYFILE]);
                let _ = st.set_meta(META_EXISTS, "1");
            }
            _ => {}
        }
        let p = Problem { kind, text, at: now() };
        let _ = st.set_meta(META_ERROR, &serde_json::to_string(&p).unwrap_or_default());
    }
    if matches!(kind, CloudFail::Gone | CloudFail::Replaced) {
        let _ = app.emit("backup", ());
    }
    emit(app, CloudEvent { running: false, received: 0, step: None });
    Some(kind)
}

fn ready(st: &Store) -> bool {
    link_of(st).is_some() && meta(st, META_PASSWORD).is_some() && meta(st, META_KEY).is_some()
}

/// Для сквозных проверок в отладочной сборке паузы короче.
fn secs(name: &str, default: u64) -> Duration {
    #[cfg(debug_assertions)]
    if let Some(s) = std::env::var(name).ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(s);
    }
    let _ = name;
    Duration::from_secs(default)
}

/// Обмен в фоне: проверяем каждые полминуты, не пора ли.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let tick = secs("POLKA_CLOUD_TICK_SECS", 30);
        let change_wait = secs("POLKA_CLOUD_CHANGE_SECS", 60);
        // Первый обмен — вскоре после запуска, чтобы не мешать открытию.
        sleep_or_wake(secs("POLKA_CLOUD_DELAY_SECS", 10));
        let mut seen: Option<u64> = None;
        let mut changed_at: Option<Instant> = None;
        let mut last_round: Option<Instant> = None;
        let mut retry_at: Option<Instant> = None;
        let mut first = true;
        loop {
            if !first {
                sleep_or_wake(tick);
            }
            first = false;
            let state = app.state::<AppState>();
            let Some((is_ready, auto, events)) = state.store.lock().ok().map(|st| (ready(&st), every(&st) == "auto", st.event_count().unwrap_or(0))) else { continue };
            let manual = MANUAL.swap(false, Ordering::SeqCst);
            let back = RESUMED.swap(false, Ordering::SeqCst);
            if !is_ready {
                last_round = None;
                retry_at = None;
                continue;
            }
            if seen.is_some_and(|s| s != events) && changed_at.is_none() {
                changed_at = Some(Instant::now());
            }
            seen.get_or_insert(events);
            let since = last_round.map(|t| t.elapsed());
            let can_retry = retry_at.is_none_or(|t| Instant::now() >= t);
            let due = manual
                || (auto && can_retry && (since.is_none_or(|d| d >= EVERY) || changed_at.is_some_and(|t| t.elapsed() >= change_wait) || (back && since.is_none_or(|d| d >= RESUME_GAP))));
            if !due {
                continue;
            }
            let failed = run_round(&app);
            last_round = Some(Instant::now());
            changed_at = None;
            seen = state.store.lock().ok().and_then(|st| st.event_count().ok()).or(Some(events));
            retry_at = match failed {
                Some(CloudFail::Offline | CloudFail::Server | CloudFail::Space | CloudFail::Format) => Some(Instant::now() + RETRY),
                Some(CloudFail::Conflict | CloudFail::Newer) => Some(Instant::now() + Duration::from_secs(6 * 3600)),
                _ => None,
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yandex_pages_open() {
        assert!(safe_page("https://id.yandex.ru/security"));
        assert!(!safe_page("http://id.yandex.ru/security"));
        assert!(!safe_page("https://id.yandex.ru.evil.example/security"));
        assert!(!safe_page("https://evil.example/?https://id.yandex.ru/"));
        assert!(!safe_page("https://id.yandex.ru/security\" onclick"));
        assert!(!safe_page("file:///etc/passwd"));
    }

    #[test]
    fn links_by_password() {
        for l in [Link::YaDisk, Link::WebDav] {
            assert_eq!(Link::parse(l.code()), Some(l));
        }
        assert_eq!(Link::parse("dropbox"), None);
        assert_eq!(Link::YaDisk.title(), "Яндекс Диск");
    }
}

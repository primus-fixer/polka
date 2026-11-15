//! Коннектор для приложения Claude на компьютере — как в «Кармане».
//!
//! Приложение Claude запускает `polka --mcp` (расширение `.mcpb`) и говорит с ним по протоколу MCP
//! через stdio. Посредник сам отвечает на `initialize` и `tools/list`, а вызовы инструментов передаёт
//! запущенной «Полке» по 127.0.0.1 — запросы из сети «Полка» не принимает. Ключ связи новый при
//! каждом запуске и лежит в папке данных; по сети он не ходит: стороны доказывают друг другу, что знают
//! его (MAC от одноразовых чисел). Программа, занявшая порт закрытой «Полки», не получит ни ключа,
//! ни запроса. Если «Полка» закрыта, посредник её запускает (на Windows).
//!
//! Чтение — когда подключение включено в настройках. Запись — только если она отдельно разрешена;
//! каждое действие Claude видно в «Полке» и отменяется кнопкой «Вернуть».

// На телефоне коннектора нет: там остаются только команды экрана.
#![cfg_attr(not(desktop), allow(unused_imports, dead_code))]

use crate::guard::LOCKED;
use crate::{err, tz_minutes, AppState, Res};
use polka_core::catalog;
use polka_core::db::Store;
use polka_core::model::Kind;
use polka_core::tools::{self, Item, Lookup};
use polka_core::{mcp, now_secs, Date};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Где запущенная «Полка» ждёт посредника: порт и ключ.
const LINK_FILE: &str = "claude.json";
const META_ON: &str = "claude.on";
const META_WRITE: &str = "claude.write";
/// Запрос посредника — одна строка JSON, не больше.
const MAX_REQUEST: u64 = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Link {
    port: u16,
    token: String,
    pid: u32,
    app: String,
}

/// Запрос посредника — после того как «Полка» доказала, что знает ключ связи.
#[derive(Serialize, Deserialize)]
struct Request {
    /// Доказательство посредника: MAC от одноразового числа «Полки».
    auth: String,
    tool: String,
    #[serde(default)]
    args: Value,
}

#[derive(Serialize, Deserialize, Default)]
struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ok: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    err: Option<String>,
}

/// Одноразовое число для проверки: 32 случайных байта.
fn nonce() -> Option<String> {
    polka_core::pairing::random_key().ok().map(|k| k.iter().map(|b| format!("{b:02x}")).collect())
}

fn valid_nonce(n: &str) -> bool {
    n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Действие Claude, которое можно отменить.
struct Action {
    id: u64,
    at: i64,
    tool: String,
    text: String,
    items: Vec<Item>,
    undone: bool,
}

static ACTIONS: Mutex<Vec<Action>> = Mutex::new(Vec::new());

/// Сегодня по местному времени (смещение присылает экран).
fn local_today(st: &Store) -> Date {
    Date::from_epoch_days((now_secs() + tz_minutes(st) * 60).div_euclid(86_400) as i32)
}

fn meta_flag(st: &Store, key: &str) -> bool {
    st.meta(key).ok().flatten().is_some_and(|v| v == "1")
}

/// Ключ без тайминговых подсказок: сравнение всегда до конца.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Окно «Полки» — на передний план: войти по PIN-коду или открыть второй запуск. На телефоне окна
/// не сворачивают — там этого нет.
#[cfg(desktop)]
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn locked(app: &AppHandle) -> String {
    #[cfg(desktop)]
    show_main(app);
    let _ = app;
    "«Полка» заблокирована: окно «Полки» открыто — войдите по PIN-коду (или Windows Hello) и повторите запрос.".into()
}

/// Найти в каталогах обложку и данные для новой записи — до записи и без блокировки базы.
fn look_up(state: &AppState, args: &Value) -> Option<Lookup> {
    let kind = match args.get("kind")?.as_str()? {
        "book" => Kind::Book,
        "film" => Kind::Film,
        "series" => Kind::Series,
        _ => return None,
    };
    let title = args.get("title")?.as_str()?.trim().to_string();
    let creators: Vec<String> = args.get("creators").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let keys = state.keys();
    let net = state.net.as_ref();
    let found = if kind == Kind::Book {
        // Название и фамилия первого автора — так каталоги находят точнее.
        let surname = creators.first().and_then(|c| c.split_whitespace().last()).unwrap_or("");
        catalog::search_books(net, format!("{title} {surname}").trim(), &keys)
    } else {
        catalog::search_films(net, &title, &keys)
    };
    let picked = tools::pick_candidate(&found.results, kind, &title, &creators)?.clone();
    let full = catalog::details(net, &picked, &keys);
    // Первая обложка, которая скачалась.
    let cover = full.covers.iter().take(3).find_map(|o| catalog::fetch_cover(net, &o.url).ok().map(|c| (c, tools::cover_label(&o.source, &o.edition))));
    let (cover, label) = match cover {
        Some((c, l)) => (Some(c), l),
        None => (None, String::new()),
    };
    Some(Lookup { candidate: full, cover, label })
}

/// Инструмент «Полки» для Claude: чтение — когда подключение включено, запись — только если она
/// разрешена; каждое записанное действие можно отменить кнопкой «Вернуть».
fn execute(app: &AppHandle, tool: &str, args: &Value) -> Result<Value, String> {
    let state = app.try_state::<AppState>().ok_or("«Полка» ещё открывается — повторите запрос через несколько секунд")?;
    let info = tools::find(tool).ok_or_else(|| format!("нет инструмента «{tool}»"))?;
    let today = {
        let st = match state.store() {
            Ok(st) => st,
            Err(e) if e == LOCKED => return Err(locked(app)),
            Err(e) => return Err(e),
        };
        if !meta_flag(&st, META_ON) {
            return Err("Подключение для Claude выключено: включите его в «Полке» — Настройки → Claude.".into());
        }
        if info.write && !meta_flag(&st, META_WRITE) {
            return Err("Запись для Claude выключена: разрешите её в «Полке» — Настройки → Claude → «Разрешить запись». Читать полку можно и так.".into());
        }
        if !info.write {
            return tools::read(&st, local_today(&st), tool, args);
        }
        let today = local_today(&st);
        // Сначала проверка без каталогов: повтор на полке или неверные данные — без лишних запросов в сеть.
        tools::plan(&st, today, tool, args, None)?;
        today
    };
    let lookup = if tool == "add_title" && args.get("look_up").and_then(|v| v.as_bool()) != Some(false) { look_up(&state, args) } else { None };
    let mut st = match state.store() {
        Ok(st) => st,
        Err(e) if e == LOCKED => return Err(locked(app)),
        Err(e) => return Err(e),
    };
    let change = tools::plan(&st, today, tool, args, lookup)?;
    tools::apply(&mut st, &change).map_err(err)?;
    drop(st);
    let id = {
        let mut list = ACTIONS.lock().map_err(err)?;
        let id = list.last().map_or(1, |a| a.id + 1);
        list.push(Action { id, at: now_secs(), tool: tool.to_string(), text: change.text.clone(), items: change.items, undone: false });
        let extra = list.len().saturating_sub(50);
        list.drain(..extra);
        id
    };
    let _ = app.emit("claude", json!({ "id": id, "text": format!("Claude: {}", change.text) }));
    let mut result = change.result;
    if let Some(m) = result.as_object_mut() {
        m.insert("done".into(), json!(change.text));
        m.insert("undo".into(), json!("Человек может отменить это в «Полке» кнопкой «Вернуть»"));
    }
    Ok(result)
}

// ——— Приём запросов посредника (компьютер) ———

/// «Полка»: посредник прислал своё одноразовое число — отвечаем доказательством и своим числом;
/// дальше принимаем запрос, только если посредник доказал, что знает ключ. `None` — связь оборвалась.
fn accept(stream: &std::net::TcpStream, token: &str) -> Option<Result<Request, String>> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST));
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let hello: Value = match serde_json::from_str(&line) {
        Ok(v) => v,
        Err(_) => return Some(Err("неверный запрос".into())),
    };
    let Some(theirs) = hello["hello"].as_str().filter(|n| valid_nonce(n)) else { return Some(Err("неверный запрос".into())) };
    let Some(mine) = nonce() else { return Some(Err("нет случайных чисел".into())) };
    let mut out = stream;
    writeln!(out, "{}", json!({ "proof": mcp::link_proof(token, "polka", theirs), "nonce": mine })).ok()?;
    line.clear();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    Some(match serde_json::from_str::<Request>(&line) {
        Ok(r) if same(&r.auth, &mcp::link_proof(token, "bridge", &mine)) => Ok(r),
        Ok(_) => Err("неверный ключ".into()),
        Err(_) => Err("неверный запрос".into()),
    })
}

#[cfg(desktop)]
fn serve(app: &AppHandle, stream: std::net::TcpStream, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let reply = match accept(&stream, token) {
        None => return,
        Some(Ok(r)) => match execute(app, &r.tool, &r.args) {
            Ok(v) => Reply { ok: Some(v), err: None },
            Err(e) => Reply { ok: None, err: Some(e) },
        },
        Some(Err(e)) => Reply { ok: None, err: Some(e) },
    };
    let mut out = &stream;
    let _ = writeln!(out, "{}", serde_json::to_string(&reply).unwrap_or_default());
}

/// При запуске на компьютере: ждём посредника на 127.0.0.1 и записываем, где нас искать.
#[cfg(desktop)]
pub fn start(app: AppHandle, data_dir: PathBuf) {
    std::thread::spawn(move || {
        let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
            Ok(l) => l,
            Err(e) => return eprintln!("коннектор для Claude: {e}"),
        };
        let Ok(port) = listener.local_addr().map(|a| a.port()) else { return };
        let Ok(key) = polka_core::pairing::random_key() else { return };
        let token: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let link = Link { port, token: token.clone(), pid: std::process::id(), app: crate::APP_VERSION.into() };
        if let Err(e) = write_link(&data_dir, &link) {
            return eprintln!("коннектор для Claude: {e}");
        }
        for stream in listener.incoming().flatten() {
            let (app, token) = (app.clone(), token.clone());
            std::thread::spawn(move || serve(&app, stream, &token));
        }
    });
}

#[cfg(desktop)]
fn write_link(dir: &Path, link: &Link) -> std::io::Result<()> {
    let path = dir.join(LINK_FILE);
    let tmp = dir.join(format!("{LINK_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(link)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)
}

// ——— Посредник: `polka --mcp` ———

/// Папка данных «Полки» — та же, что у запущенного приложения.
#[cfg(desktop)]
fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("POLKA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_default().join("io.github.primusfixer.polka")
}

#[cfg(desktop)]
enum CallError {
    /// «Полка» не запущена (или ещё не успела открыться).
    NotRunning,
    Failed(String),
}

#[cfg(desktop)]
fn call_app(dir: &Path, tool: &str, args: &Value) -> Result<Value, CallError> {
    let link: Link = std::fs::read(dir.join(LINK_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(CallError::NotRunning)?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], link.port));
    let stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| CallError::NotRunning)?;
    // Новая запись ищет обложку в каталогах — это дольше обычного запроса.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    talk(&stream, &link.token, tool, args)
}

/// Посредник: сначала «Полка» доказывает, что знает ключ связи, потом — запрос с доказательством
/// посредника. Если на порту не «Полка» (порт занят другой программой), запрос не уходит.
#[cfg(desktop)]
fn talk(stream: &std::net::TcpStream, token: &str, tool: &str, args: &Value) -> Result<Value, CallError> {
    let mine = nonce().ok_or_else(|| CallError::Failed("нет случайных чисел".into()))?;
    let mut out = stream;
    writeln!(out, "{}", json!({ "hello": mine })).map_err(|_| CallError::NotRunning)?;
    let mut reader = BufReader::new(stream.take(64 << 20));
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|_| CallError::NotRunning)?;
    let challenge: Value = serde_json::from_str(&line).map_err(|_| CallError::NotRunning)?;
    if let Some(e) = challenge["err"].as_str() {
        return Err(CallError::Failed(format!("«Полка» не приняла запрос ({e}) — перезапустите приложение Claude и «Полку»")));
    }
    let proof = challenge["proof"].as_str().unwrap_or_default();
    let Some(theirs) = challenge["nonce"].as_str().filter(|n| valid_nonce(n)) else { return Err(CallError::NotRunning) };
    if proof.is_empty() || !same(proof, &mcp::link_proof(token, "polka", &mine)) {
        // На порту не «Полка»: ни ключа, ни запроса ему не отдаём.
        return Err(CallError::NotRunning);
    }
    let req = Request { auth: mcp::link_proof(token, "bridge", theirs), tool: tool.to_string(), args: args.clone() };
    writeln!(out, "{}", serde_json::to_string(&req).map_err(|e| CallError::Failed(e.to_string()))?).map_err(|_| CallError::NotRunning)?;
    line.clear();
    reader.read_line(&mut line).map_err(|e| CallError::Failed(format!("«Полка» не ответила: {e}")))?;
    let reply: Reply = serde_json::from_str(&line).map_err(|_| CallError::Failed("«Полка» ответила непонятно — обновите её".into()))?;
    match (reply.ok, reply.err) {
        (Some(v), _) => Ok(v),
        (None, Some(e)) => Err(CallError::Failed(e)),
        _ => Err(CallError::Failed("пустой ответ «Полки»".into())),
    }
}

/// Запустить «Полку» отдельно от приложения Claude (чтобы она не закрылась вместе с ним).
#[cfg(desktop)]
fn launch() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP);
        cmd.spawn().is_ok() || {
            // Если выйти из задания приложения Claude нельзя — обычный запуск.
            let mut plain = std::process::Command::new(&exe);
            plain.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            plain.creation_flags(CREATE_NEW_PROCESS_GROUP);
            plain.spawn().is_ok()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
        false
    }
}

#[cfg(desktop)]
fn forward(dir: &Path, tool: &str, args: &Value) -> Result<Value, String> {
    match call_app(dir, tool, args) {
        Ok(v) => return Ok(v),
        Err(CallError::Failed(e)) => return Err(e),
        Err(CallError::NotRunning) => {}
    }
    // Запуск — только на Windows: второй запуск лишь открывает уже работающую «Полку».
    if !cfg!(windows) || !launch() {
        return Err("«Полка» не запущена: откройте её на этом компьютере и повторите запрос.".into());
    }
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(40) {
        std::thread::sleep(Duration::from_millis(400));
        match call_app(dir, tool, args) {
            Ok(v) => return Ok(v),
            Err(CallError::Failed(e)) => return Err(e),
            Err(CallError::NotRunning) => {}
        }
    }
    Err("«Полка» не открылась за 40 секунд: откройте её вручную и повторите запрос.".into())
}

/// `polka --mcp`: MCP по stdio для приложения Claude. Код выхода — 0, когда приложение Claude закрыло поток.
#[cfg(desktop)]
pub fn relay() -> i32 {
    let dir = data_dir();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return 1 };
        if line.trim().is_empty() {
            continue;
        }
        let reply = mcp::handle(&line, crate::APP_VERSION, &mut |tool, args| forward(&dir, tool, args));
        if let Some(r) = reply {
            if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() {
                return 1;
            }
        }
    }
    0
}

// ——— Команды экрана ———

#[derive(Serialize)]
pub struct ActionView {
    id: u64,
    at: i64,
    tool: String,
    text: String,
    undone: bool,
}

#[derive(Serialize)]
pub struct ClaudeStatus {
    /// Коннектор есть только на компьютере.
    desktop: bool,
    on: bool,
    write: bool,
    /// Чем приложение Claude запускает «Полку» — для ручной настройки.
    command: String,
    dir: String,
    /// Готовый текст для `claude_desktop_config.json`.
    config: String,
    /// Последние действия Claude, новые сверху.
    actions: Vec<ActionView>,
}

fn command_path() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

fn status(state: &AppState) -> Res<ClaudeStatus> {
    let st = state.store()?;
    let actions = ACTIONS
        .lock()
        .map_err(err)?
        .iter()
        .rev()
        .map(|a| ActionView { id: a.id, at: a.at, tool: a.tool.clone(), text: a.text.clone(), undone: a.undone })
        .collect();
    let (command, dir) = (command_path(), state.dir.display().to_string());
    let config = serde_json::to_string_pretty(&json!({ "mcpServers": { "polka": { "command": command, "args": ["--mcp"], "env": { "POLKA_DIR": dir } } } })).map_err(err)?;
    Ok(ClaudeStatus { desktop: cfg!(desktop), on: meta_flag(&st, META_ON), write: meta_flag(&st, META_WRITE), command, dir, config, actions })
}

#[tauri::command]
pub fn claude_status(state: State<AppState>) -> Res<ClaudeStatus> {
    status(&state)
}

#[tauri::command]
pub fn claude_set(state: State<AppState>, on: bool, write: bool) -> Res<ClaudeStatus> {
    if !cfg!(desktop) {
        return Err("Коннектор для Claude — в «Полке» на компьютере".into());
    }
    {
        let st = state.store()?;
        st.set_meta(META_ON, if on { "1" } else { "0" }).map_err(err)?;
        st.set_meta(META_WRITE, if on && write { "1" } else { "0" }).map_err(err)?;
    }
    status(&state)
}

/// Отменить действие Claude, если после него это никто не менял.
#[tauri::command]
pub fn claude_undo(state: State<AppState>, id: u64) -> Res<ClaudeStatus> {
    {
        let mut st = state.store()?;
        let mut list = ACTIONS.lock().map_err(err)?;
        let a = list.iter_mut().find(|a| a.id == id).ok_or("Это действие уже не найти — поправьте вручную")?;
        if a.undone {
            return Err("Уже отменено".into());
        }
        if !tools::still_current(&st, &a.items).map_err(err)? {
            return Err("После Claude запись уже меняли — отменить нельзя, поправьте вручную".into());
        }
        tools::undo(&mut st, &a.items).map_err(err)?;
        a.undone = true;
    }
    status(&state)
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// Расширение для приложения Claude — в «Загрузки»; `open` — сразу открыть: приложение Claude
/// предложит установить его.
#[tauri::command]
pub fn claude_bundle(app: AppHandle, state: State<AppState>, open: bool) -> Res<String> {
    if !cfg!(desktop) {
        return Err("Коннектор для Claude — в «Полке» на компьютере".into());
    }
    let manifest = mcp::manifest(crate::APP_VERSION, &command_path(), &state.dir.display().to_string(), platform());
    let data = mcp::bundle(&manifest, include_bytes!("../icons/icon.png"))?;
    let dir = app.path().download_dir().map_err(|e| format!("не найти папку «Загрузки»: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let path = dir.join("Полка для Claude.mcpb");
    std::fs::write(&path, &data).map_err(|e| format!("расширение не сохранилось: {e}"))?;
    if open {
        use tauri_plugin_opener::OpenerExt;
        app.opener().open_path(path.display().to_string(), None::<&str>).map_err(|e| format!("Файл сохранён в «Загрузки», но не открылся: {e}. Установите его в приложении Claude: Настройки → Extensions → Install Extension…"))?;
    }
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd") && !same("abc", "abcd"));
    }

    /// «Полка» на 127.0.0.1: принимает одно подключение и отвечает эхом запроса.
    fn fake_polka(token: &'static str) -> (u16, std::thread::JoinHandle<Option<Result<Request, String>>>) {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (stream, _) = l.accept().unwrap();
            let got = accept(&stream, token);
            let reply = match &got {
                Some(Ok(r)) => Reply { ok: Some(json!({ "tool": r.tool, "args": r.args })), err: None },
                Some(Err(e)) => Reply { ok: None, err: Some(e.clone()) },
                None => return got,
            };
            let mut out = &stream;
            writeln!(out, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
            got
        });
        (port, h)
    }

    fn connect(port: u16) -> std::net::TcpStream {
        let s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    }

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[cfg(desktop)]
    #[test]
    fn bridge_and_polka_prove_the_key_without_sending_it() {
        let (port, h) = fake_polka(KEY);
        let v = talk(&connect(port), KEY, "summary", &json!({ "x": 1 })).ok().unwrap();
        assert_eq!(v, json!({ "tool": "summary", "args": { "x": 1 } }));
        assert!(matches!(h.join().unwrap(), Some(Ok(_))));
        // Посредник с чужим ключом: доказательство «Полки» не сходится — посредник сам ничего не отправил.
        let (port, h) = fake_polka(KEY);
        let r = talk(&connect(port), &"f".repeat(64), "summary", &json!({}));
        assert!(matches!(r, Err(CallError::NotRunning)));
        assert!(h.join().unwrap().is_none(), "запрос не ушёл");
    }

    #[cfg(desktop)]
    #[test]
    fn a_program_on_the_port_gets_neither_key_nor_request() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (stream, _) = l.accept().unwrap();
            let mut seen = String::new();
            let mut reader = BufReader::new(&stream);
            reader.read_line(&mut seen).unwrap();
            // Самозванец отвечает как умеет — ключа он не знает.
            let mut out = &stream;
            writeln!(out, "{}", json!({ "proof": "00".repeat(32), "nonce": "11".repeat(32) })).unwrap();
            let mut rest = String::new();
            let _ = reader.read_line(&mut rest);
            seen + &rest
        });
        let r = talk(&connect(port), KEY, "add_title", &json!({ "title": "Щегол" }));
        assert!(matches!(r, Err(CallError::NotRunning)));
        let seen = h.join().unwrap();
        assert!(seen.contains("hello") && !seen.contains(KEY) && !seen.contains("Щегол") && !seen.contains("auth"), "{seen}");
    }

    #[test]
    fn a_bridge_without_the_key_is_refused() {
        let (port, h) = fake_polka(KEY);
        let s = connect(port);
        let mut out = &s;
        // Сразу запрос, без приветствия и доказательства.
        writeln!(out, "{}", json!({ "auth": "00", "tool": "summary", "args": {} })).unwrap();
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).unwrap();
        assert!(line.contains("неверный запрос"), "{line}");
        assert!(matches!(h.join().unwrap(), Some(Err(_))));
    }

    /// Посредник находит «Полку» по файлу связи в папке данных и получает ответ инструмента из базы.
    #[cfg(desktop)]
    #[test]
    fn relay_reads_the_shelf_through_the_link_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = Store::open(&dir.path().join("t.db"), &[5; 32], "dev1").unwrap();
        let mut t = polka_core::model::Title::new(Kind::Book, "Лавр", Date::parse("2026-10-10").unwrap());
        t.id = "lavr".into();
        st.put(polka_core::model::KIND, "lavr", &t).unwrap();
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let link = Link { port: l.local_addr().unwrap().port(), token: KEY.into(), pid: 1, app: "test".into() };
        write_link(dir.path(), &link).unwrap();
        let h = std::thread::spawn(move || {
            let (stream, _) = l.accept().unwrap();
            let r = accept(&stream, KEY).unwrap().unwrap();
            let v = tools::read(&st, Date::parse("2026-11-15").unwrap(), &r.tool, &r.args).unwrap();
            let mut out = &stream;
            writeln!(out, "{}", serde_json::to_string(&Reply { ok: Some(v), err: None }).unwrap()).unwrap();
        });
        let v = call_app(dir.path(), "shelf", &json!({ "status": "want" })).ok().unwrap();
        h.join().unwrap();
        assert_eq!(v["total"], 1);
        assert_eq!(v["titles"][0]["title"], "Лавр");
        // Файла связи нет — «Полка» не запущена.
        assert!(matches!(call_app(&dir.path().join("нет"), "summary", &json!({})), Err(CallError::NotRunning)));
    }

    #[cfg(desktop)]
    #[test]
    fn relay_finds_the_data_folder() {
        std::env::set_var("POLKA_DIR", "/tmp/polka-test-dir");
        assert_eq!(data_dir(), PathBuf::from("/tmp/polka-test-dir"));
        std::env::remove_var("POLKA_DIR");
        assert!(data_dir().ends_with("io.github.primusfixer.polka"));
    }
}

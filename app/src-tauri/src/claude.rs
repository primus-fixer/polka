//! Коннектор для приложения Claude на компьютере.
//!
//! Приложение Claude запускает `polka --mcp` (расширение `.mcpb`) и говорит с ним по протоколу MCP
//! через stdio. Посредник сам отвечает на `initialize` и `tools/list`, а вызовы инструментов передаёт
//! запущенной «Полке» по 127.0.0.1 — запросы из сети «Полка» не принимает. Ключ связи новый при
//! каждом запуске и лежит в папке данных; по сети он не ходит: стороны доказывают друг другу, что знают
//! его (MAC от одноразовых чисел). Если «Полка» закрыта, посредник её запускает (на Windows).
//! Посредник и приём запросов — общие из kit (`kit_core::link`); здесь — инструменты «Полки».
//!
//! Чтение — когда подключение включено в настройках. Запись — только если она отдельно разрешена;
//! каждое действие Claude видно в «Полке» и отменяется кнопкой «Вернуть».

// На телефоне коннектора нет: там остаются только команды экрана.
#![cfg_attr(not(desktop), allow(unused_imports, dead_code))]

use kit_tauri::guard::LOCKED;
use crate::{err, tz_minutes, AppState, Res};
use polka_core::catalog;
use polka_core::db::Store;
use polka_core::model::Kind;
use polka_core::tools::{self, Item, Lookup};
use kit_core::link::{self, Connector};
use polka_core::{mcp, now_secs, Date};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

const META_ON: &str = "claude.on";
const META_WRITE: &str = "claude.write";
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

// ——— Связь с посредником `polka --mcp` (компьютер) ———

/// Коннектор «Полки» для связи посредника с запущенной «Полкой». Идентификатор — имя папки данных
/// (`identifier` в `tauri.conf.json`); его, как и строку ключа связи, менять нельзя.
pub const CONNECTOR: Connector = Connector { server: mcp::SERVER, extension: mcp::EXTENSION, names: crate::names::APP, identifier: "io.github.primusfixer.polka" };

/// При запуске на компьютере — после того как база открыта: ждём посредника на 127.0.0.1 и записываем,
/// где нас искать.
#[cfg(desktop)]
pub fn start(app: AppHandle, data_dir: PathBuf) {
    if let Err(e) = link::listen(&CONNECTOR, &data_dir, crate::APP_VERSION, move |tool, args| execute(&app, tool, args)) {
        eprintln!("коннектор для Claude: {e}");
    }
}

/// `polka --mcp`: MCP по stdio для приложения Claude. Код выхода — 0, когда приложение Claude закрыло поток.
#[cfg(desktop)]
pub fn relay() -> i32 {
    link::relay(&CONNECTOR, &mcp::tools(), crate::APP_VERSION)
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

    /// Строки связи — те же, что у «Полки» до kit: посредник прежней версии найдёт папку данных
    /// и узнает запущенную «Полку».
    #[test]
    fn connector_is_the_same_as_before_kit() {
        assert_eq!(CONNECTOR.server.name, "polka");
        assert_eq!(CONNECTOR.server.link_personal, b"polka-link");
        assert_eq!(CONNECTOR.extension.dir_env, "POLKA_DIR");
        assert_eq!(CONNECTOR.identifier, "io.github.primusfixer.polka");
        assert_eq!(link::LINK_FILE, "claude.json");
        assert!(CONNECTOR.names.feminine, "«Полка» не запущена, откройте её");
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
        let st = Mutex::new(st);
        link::listen(&CONNECTOR, dir.path(), "test", move |tool, args| tools::read(&st.lock().unwrap(), Date::parse("2026-11-15").unwrap(), tool, args).map_err(|e| e.to_string())).unwrap();
        let v = link::forward(&CONNECTOR, dir.path(), "shelf", &json!({ "status": "want" })).unwrap();
        assert_eq!(v["total"], 1);
        assert_eq!(v["titles"][0]["title"], "Лавр");
        // Файла связи нет — «Полка» не запущена.
        assert_eq!(link::forward(&CONNECTOR, &dir.path().join("нет"), "summary", &json!({})), Err("«Полка» не запущена: откройте её на этом компьютере и повторите запрос.".into()));
    }

    #[cfg(desktop)]
    #[test]
    fn relay_finds_the_data_folder() {
        std::env::set_var("POLKA_DIR", "/tmp/polka-test-dir");
        assert_eq!(link::data_dir(&CONNECTOR), PathBuf::from("/tmp/polka-test-dir"));
        std::env::remove_var("POLKA_DIR");
        assert!(link::data_dir(&CONNECTOR).ends_with("io.github.primusfixer.polka"));
    }
}

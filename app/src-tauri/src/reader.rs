//! Читалка: файлы книг на этом устройстве, главы для экрана, место чтения.
//!
//! Файлы лежат в папке данных приложения (`books/<sha256>.<ext>`), связь файла с записью полки —
//! в местных настройках базы (`book.<sha256>`): файлы между устройствами не ходят. Место чтения —
//! запись журнала `reading`: уходит на другие устройства вместе с полкой, и там книга (тот же
//! файл — по тому же хешу) открывается на том же месте. Картинки глав отдаёт протокол `book`
//! (`book://localhost/<sha256>/<путь в книге>`, на Windows и Android — `http://book.localhost/…`).

use crate::{err, get_title, put_title, tz_minutes, AppState, Res};
use polka_core::db::{blob_id, Store};
use polka_core::model::{Title, KIND};
use polka_core::reader::{percent_decode, shelf, Book, BookFormat, Chapter, Hit, Mark, Meta, Reading, TocItem, MARK, MAX_BOOK, READING};
use polka_core::{new_id, now_secs, Date};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// Открытая книга — одна, последняя: главы и картинки берутся из неё.
#[derive(Default)]
pub(crate) struct Books {
    open: Mutex<Option<(String, Arc<Book>)>>,
}

/// Файл книги на этом устройстве.
#[derive(Clone, Serialize, Deserialize)]
struct Local {
    /// Запись полки.
    title: String,
    /// Имя файла, как его выбрали.
    name: String,
    format: BookFormat,
    size: u64,
    added: i64,
}

#[derive(Serialize)]
pub struct LocalBook {
    hash: String,
    title: String,
    name: String,
    format: BookFormat,
    size: u64,
    added: i64,
}

#[derive(Serialize)]
pub struct BookView {
    hash: String,
    format: BookFormat,
    meta: Meta,
    chapters: Vec<Chapter>,
    toc: Vec<TocItem>,
    total: usize,
    /// Запись полки; если она изменилась при открытии («Хочу» → «Читаю») — уже новая.
    title: Option<Title>,
}

#[derive(Clone, Serialize)]
pub struct Imported {
    hash: String,
    title: Title,
    /// Запись заведена только что (а не найдена на полке).
    created: bool,
}

const META: &str = "book.";

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn books_dir(state: &AppState) -> PathBuf {
    state.dir.join("books")
}

const EXTS: [&str; 3] = ["epub", "fb2", "fb2.zip"];

fn file_of(state: &AppState, hash: &str) -> Option<PathBuf> {
    let dir = books_dir(state);
    EXTS.iter().map(|e| dir.join(format!("{hash}.{e}"))).find(|p| p.is_file())
}

/// Сегодня по местному времени (смещение присылает экран).
fn local_today(st: &Store) -> Date {
    let secs = now_secs() + tz_minutes(st) * 60;
    Date::from_epoch_days(secs.div_euclid(86_400) as i32)
}

fn local(st: &Store, hash: &str) -> Option<Local> {
    st.meta(&format!("{META}{hash}")).ok().flatten().and_then(|v| serde_json::from_str(&v).ok())
}

/// Имя файла для показа: без пути и управляющих знаков.
fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.chars().filter(|c| !c.is_control()).take(200).collect::<String>().trim().to_string()
}

/// Книга по хешу: из памяти или с диска.
fn load(app: &AppHandle, hash: &str) -> Res<Arc<Book>> {
    if !is_hash(hash) {
        return Err("неверный номер книги".into());
    }
    let books = app.state::<Books>();
    if let Some((h, b)) = books.open.lock().map_err(err)?.as_ref() {
        if h == hash {
            return Ok(b.clone());
        }
    }
    let state = app.state::<AppState>();
    let path = file_of(&state, hash).ok_or("Файла книги нет на этом устройстве")?;
    let data = std::fs::read(&path).map_err(|e| format!("файл книги не прочитался: {e}"))?;
    let book = Arc::new(Book::open(data).map_err(err)?);
    *books.open.lock().map_err(err)? = Some((hash.to_string(), book.clone()));
    Ok(book)
}

/// Книга из файла: проверить, сохранить у себя, найти запись на полке или завести новую.
/// `prefer` — запись, из карточки которой открыли файл: привязать к ней.
fn import(app: &AppHandle, data: Vec<u8>, name: &str, today: Option<Date>, prefer: Option<String>) -> Res<Imported> {
    if data.len() > MAX_BOOK {
        return Err(format!("Книга больше {} МБ — такую не открыть", MAX_BOOK >> 20));
    }
    let hash = blob_id(&data);
    let zipped = data.starts_with(b"PK\x03\x04");
    let size = data.len() as u64;
    let book = Book::open(data.clone()).map_err(err)?;
    let state = app.state::<AppState>();
    let dir = books_dir(&state);
    std::fs::create_dir_all(&dir).map_err(err)?;
    if file_of(&state, &hash).is_none() {
        let ext = match (book.format, zipped) {
            (BookFormat::Epub, _) => "epub",
            (BookFormat::Fb2, false) => "fb2",
            (BookFormat::Fb2, true) => "fb2.zip",
        };
        let tmp = dir.join(format!("{hash}.part"));
        std::fs::write(&tmp, &data).map_err(|e| format!("книга не сохранилась: {e}"))?;
        std::fs::rename(&tmp, dir.join(format!("{hash}.{ext}"))).map_err(|e| format!("книга не сохранилась: {e}"))?;
    }
    drop(data);
    let chars = book.total();
    let mut st = state.store()?;
    let today = today.unwrap_or_else(|| local_today(&st));
    let exists = |st: &Store, id: &str| st.get::<Title>(KIND, id).ok().flatten().is_some();
    let chosen = prefer.filter(|id| exists(&st, id));
    let linked = || local(&st, &hash).map(|l| l.title).filter(|id| exists(&st, id));
    // Тот же файл уже читали на другом устройстве — место пришло с журналом.
    let read_elsewhere = || st.list::<Reading>(READING).ok().and_then(|list| list.into_iter().find(|r| r.book == hash).map(|r| r.title)).filter(|id| exists(&st, id));
    let found = match chosen.or_else(linked).or_else(read_elsewhere) {
        Some(id) => Some(id),
        None => shelf::find(&st.list::<Title>(KIND).map_err(err)?, &book.meta),
    };
    let (title, created) = match found {
        Some(id) => {
            let mut t = get_title(&st, &id)?;
            let mut changed = shelf::start_reading(&mut t, chars, today);
            if t.cover.is_none() {
                if let Some((mime, img)) = book.cover() {
                    t.cover = Some(st.put_blob(mime, &img).map_err(err)?);
                    t.cover_source = Some("из файла книги".into());
                    changed = true;
                }
            }
            (if changed { put_title(&mut st, t)? } else { t }, false)
        }
        None => {
            let mut t = shelf::new_title(&book.meta, chars, name, today);
            t.id = new_id().map_err(err)?;
            if let Some((mime, img)) = book.cover() {
                t.cover = Some(st.put_blob(mime, &img).map_err(err)?);
                t.cover_source = Some("из файла книги".into());
            }
            (put_title(&mut st, t)?, true)
        }
    };
    let entry = Local { title: title.id.clone(), name: clean_name(name), format: book.format, size, added: now_secs() };
    st.set_meta(&format!("{META}{hash}"), &serde_json::to_string(&entry).map_err(err)?).map_err(err)?;
    drop(st);
    *app.state::<Books>().open.lock().map_err(err)? = Some((hash.clone(), Arc::new(book)));
    Ok(Imported { hash, title, created })
}

/// Книга, загруженная экраном кусками (как файл копии): выбрали файл на компьютере или телефоне.
#[tauri::command]
pub async fn book_import(app: AppHandle, upload: String, name: String, title: Option<String>, today: String) -> Res<Imported> {
    let today = crate::day(&today)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = crate::backup::upload_path(&app.state::<AppState>(), &upload)?;
        let data = std::fs::read(&path).map_err(|_| "Файл не загрузился — выберите его снова".to_string());
        let _ = std::fs::remove_file(&path);
        import(&app, data?, &name, Some(today), title)
    })
    .await
    .map_err(err)?
}

/// Android: книгу открыли «С помощью…» или отправили через «Поделиться» — файл уже скопирован
/// в папку `incoming` кеша приложения.
#[tauri::command]
pub async fn book_import_incoming(app: AppHandle, name: String, today: String) -> Res<Imported> {
    let today = crate::day(&today)?;
    if name.is_empty() || name.len() > 200 || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("неверное имя файла".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let path = app.path().app_cache_dir().map_err(err)?.join("incoming").join(&name);
        let data = std::fs::read(&path).map_err(|_| "Файл книги не пришёл — откройте его снова".to_string());
        let _ = std::fs::remove_file(&path);
        import(&app, data?, &name, Some(today), None)
    })
    .await
    .map_err(err)?
}

/// Компьютер: файл книги перетащили в окно.
pub fn dropped(app: &AppHandle, paths: &[PathBuf]) {
    let is_book = |p: &Path| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        n.ends_with(".epub") || n.ends_with(".fb2") || n.ends_with(".fb2.zip") || n.ends_with(".zip")
    };
    let Some(path) = paths.iter().find(|p| is_book(p)).cloned() else { return };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let result = std::fs::metadata(&path)
            .map_err(err)
            .and_then(|m| if m.len() as usize > MAX_BOOK { Err(format!("Книга больше {} МБ — такую не открыть", MAX_BOOK >> 20)) } else { Ok(()) })
            .and_then(|_| std::fs::read(&path).map_err(|e| format!("файл не прочитался: {e}")))
            .and_then(|data| import(&app, data, &name, None, None));
        let _ = match result {
            Ok(imported) => app.emit("book", serde_json::json!({ "ok": imported })),
            Err(e) => app.emit("book", serde_json::json!({ "error": e })),
        };
    });
}

/// Книга для читалки: описание, главы, оглавление. Открыть книгу — начать читать:
/// запись «Хочу» становится «В процессе».
#[tauri::command]
pub async fn book_open(app: AppHandle, hash: String, today: String) -> Res<BookView> {
    let today = crate::day(&today)?;
    tauri::async_runtime::spawn_blocking(move || {
        let book = load(&app, &hash)?;
        let state = app.state::<AppState>();
        let mut st = state.store()?;
        let title = match local(&st, &hash).and_then(|l| st.get::<Title>(KIND, &l.title).ok().flatten()) {
            Some(mut t) => Some(if shelf::start_reading(&mut t, book.total(), today) { put_title(&mut st, t)? } else { t }),
            None => None,
        };
        Ok(BookView { hash, format: book.format, meta: book.meta.clone(), chapters: book.chapters.clone(), toc: book.toc.clone(), total: book.total(), title })
    })
    .await
    .map_err(err)?
}

/// Глава — JSON дерева для страницы (см. `polka_core::reader::tree`).
#[tauri::command]
pub async fn book_chapter(app: AppHandle, hash: String, index: usize) -> Res<tauri::ipc::Response> {
    tauri::async_runtime::spawn_blocking(move || {
        let book = load(&app, &hash)?;
        let json = book.chapter(index).ok_or("нет такой главы")?;
        Ok(tauri::ipc::Response::new(json.to_string()))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn book_search(app: AppHandle, hash: String, query: String) -> Res<Vec<Hit>> {
    if query.chars().count() > 200 {
        return Err("слишком длинный запрос".into());
    }
    tauri::async_runtime::spawn_blocking(move || Ok(load(&app, &hash)?.search(&query, 300))).await.map_err(err)?
}

/// Файлы книг на этом устройстве.
#[tauri::command]
pub fn book_files(state: State<'_, AppState>) -> Res<Vec<LocalBook>> {
    let st = state.store()?;
    let mut out = Vec::new();
    for (key, value) in st.meta_prefix(META).map_err(err)? {
        let hash = &key[META.len()..];
        let Ok(l) = serde_json::from_str::<Local>(&value) else { continue };
        if is_hash(hash) && file_of(&state, hash).is_some() {
            out.push(LocalBook { hash: hash.to_string(), title: l.title, name: l.name, format: l.format, size: l.size, added: l.added });
        }
    }
    Ok(out)
}

/// Убрать файл книги с этого устройства (запись на полке и место чтения остаются).
#[tauri::command]
pub fn book_forget(app: AppHandle, hash: String) -> Res<()> {
    if !is_hash(&hash) {
        return Err("неверный номер книги".into());
    }
    let state = app.state::<AppState>();
    if let Some(path) = file_of(&state, &hash) {
        std::fs::remove_file(path).map_err(|e| format!("файл не удалился: {e}"))?;
    }
    state.store()?.delete_meta(&format!("{META}{hash}")).map_err(err)?;
    let books = app.state::<Books>();
    let mut open = books.open.lock().map_err(err)?;
    if open.as_ref().is_some_and(|(h, _)| *h == hash) {
        *open = None;
    }
    Ok(())
}

#[tauri::command]
pub fn reading_list(state: State<'_, AppState>) -> Res<Vec<Reading>> {
    state.store()?.list::<Reading>(READING).map_err(err)
}

/// Место чтения — в журнал (экран пишет его не чаще раза в пару минут и при выходе из книги).
#[tauri::command]
pub fn reading_save(state: State<'_, AppState>, reading: Reading) -> Res<()> {
    reading.check().map_err(err)?;
    let mut st = state.store()?;
    get_title(&st, &reading.title)?;
    st.put(READING, &reading.title, &reading).map_err(err)
}

/// Закладки и цитаты: все или одной записи полки — по порядку в книге.
#[tauri::command]
pub fn marks(state: State<'_, AppState>, title: Option<String>) -> Res<Vec<Mark>> {
    let mut list = state.store()?.list::<Mark>(MARK).map_err(err)?;
    if let Some(t) = title {
        list.retain(|m| m.title == t);
    }
    list.sort_by(|a, b| (a.title.as_str(), a.chapter, a.offset, a.created).cmp(&(b.title.as_str(), b.chapter, b.offset, b.created)));
    Ok(list)
}

/// Новая закладка или цитата (без номера) или правка заметки к цитате.
#[tauri::command]
pub fn mark_save(state: State<'_, AppState>, mark: Mark) -> Res<Mark> {
    let mut m = mark;
    if m.id.is_empty() {
        m.id = new_id().map_err(err)?;
    }
    if m.created == 0 {
        m.created = now_secs();
    }
    m.text = m.text.trim().to_string();
    m.note = m.note.trim().to_string();
    m.check().map_err(err)?;
    let mut st = state.store()?;
    get_title(&st, &m.title)?;
    st.put(MARK, &m.id, &m).map_err(err)?;
    Ok(m)
}

#[tauri::command]
pub fn mark_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    let mut st = state.store()?;
    if st.get::<Mark>(MARK, &id).map_err(err)?.is_none() {
        return Ok(());
    }
    st.delete(MARK, &id).map_err(err)
}

/// Внешняя ссылка из книги — в браузере; только http, https и mailto.
#[tauri::command]
pub fn open_link(app: AppHandle, url: String) -> Res<()> {
    let lower = url.trim().to_ascii_lowercase();
    if url.len() > 2000 || !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return Err("такую ссылку не открыть".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url.trim(), None::<&str>).map_err(err)
}

/// Протокол `book`: картинка книги по хешу файла и пути внутри книги.
pub fn response(app: &AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let raw = percent_decode(path.trim_start_matches('/'));
    let found = raw.split_once('/').filter(|(hash, _)| is_hash(hash)).and_then(|(hash, inner)| load(app, hash).ok().and_then(|b| b.image(inner)));
    let builder = tauri::http::Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        // Если такой адрес когда-нибудь откроют как страницу — скрипты в нём (SVG) не запустятся.
        .header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; img-src data:")
        .header("X-Content-Type-Options", "nosniff");
    let resp = match found {
        // Хеш файла и путь — содержимое по этому адресу не меняется.
        Some((mime, data)) => builder.status(200).header("Content-Type", mime).header("Cache-Control", "max-age=31536000, immutable").body(data),
        None => builder.status(404).body(Vec::new()),
    };
    resp.unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_hashes() {
        assert_eq!(clean_name("C:\\Книги\\Пикник.fb2"), "Пикник.fb2");
        assert_eq!(clean_name("/home/a/b.epub\u{0}"), "b.epub");
        assert!(is_hash(&"a".repeat(64)));
        assert!(!is_hash(&"A".repeat(64)));
        assert!(!is_hash("../x"));
    }
}

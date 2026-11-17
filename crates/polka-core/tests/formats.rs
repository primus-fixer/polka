//! Форматы «Полки» закреплены образцами.
//!
//! В `tests/fixtures/v<версия>/` лежит то, что сделала эта версия «Полки»: зашифрованная база, резервная
//! копия (с файлом книги), папка облака (с файлом книги) и строки: QR-код связи, ключи копий и облака,
//! доказательство коннектора. Проверки ниже читают образцы **всех** версий текущим кодом: если после правки
//! старый файл перестал открываться или читается иначе — проверка падает.
//!
//! Образцы не пересоздают. Новая версия добавляет свою папку рядом — `make_fixtures` ниже
//! (запускается руками), старые папки остаются как есть.

use polka_core::backup::{self, BackupKey, Kdf, MadeBy, Reader};
use polka_core::cloud::{self, CloudKey, Dir, MemCloud, Round, State};
use polka_core::db::{blob_id, Store};
use polka_core::files::{self, BookFile, BACKUP_KEY, FILE, SYNC_KEY};
use polka_core::model::{Format, Kind, Season, Seen, Status, Title, KIND};
use polka_core::pairing::{self, Invite};
use polka_core::reader::{BookFormat, Mark, Reading, MARK, READING};
use polka_core::stats::{Goal, GOAL_KIND};
use polka_core::{mcp, Date};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// Ключ базы-образца, её устройство и пароль копий и облака.
const DB_KEY: [u8; 32] = [0x15; 32];
const DEVICE: &str = "aaaa";
const PASSWORD: &str = "книги на даче";
/// «Сейчас» для образцов: 17 ноября 2026 года.
const NOW: i64 = 1_794_900_000;
const DIRS: [Dir; 4] = [Dir::Root, Dir::Events, Dir::Blobs, Dir::Books];
const SETTING: &str = "setting";
/// Виды записей, которые лежат в образце.
const KINDS: [&str; 6] = [KIND, GOAL_KIND, SETTING, FILE, READING, MARK];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// Папки образцов всех версий.
fn versions() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(fixtures())
        .expect("нет папки образцов")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('v')))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "образцов нет");
    out
}

fn info(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("info.json")).unwrap()).unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Всё из `old` есть в `new` с теми же значениями. Новые поля в `new` допустимы: версии добавляют
/// поля, а убирать или менять прежние нельзя.
fn assert_kept(old: &Value, new: &Value, path: &str) {
    match (old, new) {
        (Value::Object(o), Value::Object(n)) => {
            for (k, v) in o {
                let p = format!("{path}/{k}");
                assert_kept(v, n.get(k).unwrap_or_else(|| panic!("пропало: {p}")), &p);
            }
        }
        (Value::Array(o), Value::Array(n)) => {
            assert_eq!(o.len(), n.len(), "другая длина списка: {path}");
            for (i, (a, b)) in o.iter().zip(n).enumerate() {
                assert_kept(a, b, &format!("{path}/{i}"));
            }
        }
        // Числа сравниваем как числа: 9 и 9.0 — одно и то же.
        (Value::Number(a), Value::Number(b)) => assert_eq!(a.as_f64(), b.as_f64(), "изменилось: {path}"),
        _ => assert_eq!(old, new, "изменилось: {path}"),
    }
}

/// Записи из образца — те же и читаются нынешними типами (запись, которую тип не понимает, в списках
/// пропускается молча — поэтому сверяем и число записей каждого вида).
fn assert_same_records(expected: &Value, store: &Store, what: &str) {
    let records = expected["records"].as_object().unwrap();
    for kind in KINDS {
        let want = records[kind].as_object().unwrap_or_else(|| panic!("в образце нет вида {kind}"));
        for (id, body) in want {
            let got: Value = store.get(kind, id).unwrap().unwrap_or_else(|| panic!("{what}: нет записи {kind}/{id}"));
            assert_kept(body, &got, &format!("{what}/{kind}/{id}"));
        }
        let typed = match kind {
            KIND => store.list::<Title>(kind).unwrap().len(),
            GOAL_KIND => store.list::<Goal>(kind).unwrap().len(),
            SETTING => store.list::<String>(kind).unwrap().len(),
            FILE => store.list::<BookFile>(kind).unwrap().len(),
            READING => store.list::<Reading>(kind).unwrap().len(),
            MARK => store.list::<Mark>(kind).unwrap().len(),
            _ => unreachable!(),
        };
        assert_eq!(typed, want.len(), "{what}: записей вида {kind} читается не столько, сколько в образце");
    }
    // Удалённая запись так и осталась удалённой.
    assert!(store.get::<Value>(KIND, "gone").unwrap().is_none(), "{what}: удалённая запись вернулась");
    for (id, size) in expected["blobs"].as_object().unwrap() {
        let (_, data) = store.blob(id).unwrap().unwrap_or_else(|| panic!("{what}: нет снимка {id}"));
        assert_eq!(json!(data.len()), *size);
        assert_eq!(blob_id(&data), *id);
    }
}

/// Файл книги лёг на устройство целиком.
fn assert_book_here(expected: &Value, store: &Store, what: &str) {
    let id = expected["book"]["id"].as_str().unwrap();
    let path = files::path_in(store.books_dir().unwrap(), id).unwrap_or_else(|| panic!("{what}: файла книги нет"));
    let data = fs::read(path).unwrap();
    assert_eq!(blob_id(&data), id, "{what}: файл книги не тот");
    assert_eq!(json!(data.len()), expected["book"]["size"]);
}

fn cloud_from(dir: &Path) -> MemCloud {
    let cloud = MemCloud::default();
    for d in DIRS {
        let folder = dir.join("cloud").join(d.folder());
        for entry in fs::read_dir(&folder).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file()) {
            cloud.set_raw(d, entry.file_name().to_str().unwrap(), fs::read(entry.path()).unwrap());
        }
    }
    cloud
}

/// Новое устройство с папкой книг и включённым обменом файлами и копией файлов.
fn fresh(dir: &Path, name: &str, device: &str) -> Store {
    let mut s = Store::open(&dir.join(format!("{name}.db")), &[0x33; 32], device).unwrap();
    s.set_books_dir(dir.join(format!("{name}-books")));
    s.set_meta(SYNC_KEY, "1").unwrap();
    s
}

#[test]
fn old_database_opens() {
    for dir in versions() {
        let info = info(&dir);
        let tmp = tempfile::tempdir().unwrap();
        // Образец не трогаем: открываем копию.
        let db = tmp.path().join("polka.db");
        fs::copy(dir.join("store.db"), &db).unwrap();
        let store = Store::open(&db, &DB_KEY, DEVICE).unwrap_or_else(|e| panic!("{}: база не открылась: {e}", dir.display()));
        assert_same_records(&info, &store, "база");
        assert_eq!(json!(store.versions().unwrap()), info["versions"]);
        assert_eq!(json!(store.event_count().unwrap()), info["events"]);
        assert!(store.missing_blobs().unwrap().is_empty());
        drop(store);
        assert!(Store::open(&db, &[0x16; 32], DEVICE).is_err(), "с чужим ключом база не открывается");
    }
}

#[test]
fn old_backup_restores() {
    for dir in versions() {
        let info = info(&dir);
        let file = fs::read(dir.join("backup.polka")).unwrap();
        assert!(Reader::open(file.as_slice(), "другой пароль копий").is_err());
        let checked = backup::verify(Reader::open(file.as_slice(), PASSWORD).unwrap(), "bbbb", |_| Ok(false)).unwrap();
        assert_eq!(checked.manifest.app, info["version"].as_str().unwrap());
        assert_eq!(checked.manifest.device_id, DEVICE);
        assert_eq!(json!(checked.events), info["events"]);
        assert_eq!((checked.blobs as usize, checked.books), (info["blobs"].as_object().unwrap().len(), 1));

        let tmp = tempfile::tempdir().unwrap();
        let mut fresh = fresh(tmp.path(), "b", "bbbb");
        let restored = backup::restore_into(Reader::open(file.as_slice(), PASSWORD).unwrap(), &mut fresh).unwrap();
        assert_eq!(json!(restored.events), info["events"]);
        assert_eq!(restored.books, 1, "файл книги из копии лёг на устройство");
        assert_same_records(&info, &fresh, "копия");
        assert_book_here(&info, &fresh, "копия");
        assert_eq!(json!(fresh.versions().unwrap()), info["versions"]);

        // Ключ копий, как он лежит в базе устройства, по-прежнему открывает копию без пароля.
        let key = BackupKey::from_meta(info["strings"]["backup_key"].as_str().unwrap()).unwrap();
        assert!(key.matches(PASSWORD).unwrap());
        assert!(!key.matches("другой пароль копий").unwrap());
        assert!(Reader::open_with_key(file.as_slice(), &key).is_ok());
    }
}

#[test]
fn old_cloud_is_readable() {
    for dir in versions() {
        let info = info(&dir);
        let cloud = cloud_from(&dir);
        assert!(matches!(cloud::join(&cloud, "другой пароль копий", Kdf::DEFAULT, NOW), Err(polka_core::Error::CloudPassword)));
        let (key, created) = cloud::join(&cloud, PASSWORD, Kdf::DEFAULT, NOW).unwrap();
        assert!(!created, "облако уже начато — новый ключ не создаётся");
        assert_eq!(key.group(), info["strings"]["cloud_group"].as_str().unwrap());
        // Ключ облака, как он лежит в базе устройства, — тот же.
        let stored = CloudKey::from_meta(info["strings"]["cloud_key"].as_str().unwrap()).unwrap();
        assert_eq!(stored.to_meta(), key.to_meta());

        // Имена файлов облака — прежние: по ним новое устройство находит старые файлы.
        let names: Vec<Vec<String>> = DIRS.iter().map(|d| cloud.names(*d)).collect();
        assert_eq!(json!(names), info["cloud_files"]);

        let tmp = tempfile::tempdir().unwrap();
        let fresh = Mutex::new(fresh(tmp.path(), "c", "cccc"));
        let cancel = AtomicBool::new(false);
        let report = cloud::sync(&Round { cloud: &cloud, store: &fresh, key: &key, now: NOW + 60, cancel: &cancel }, &mut State::default(), &mut |_| {}).unwrap();
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(json!(report.received), info["events"]);
        assert_eq!(report.files_received, info["blobs"].as_object().unwrap().len());
        assert_eq!(report.books_received, 1, "файл книги пришёл из облака");
        let fresh = fresh.into_inner().unwrap();
        assert_same_records(&info, &fresh, "облако");
        assert_book_here(&info, &fresh, "облако");
        assert_eq!(json!(fresh.versions().unwrap()), info["versions"]);
    }
}

#[test]
fn old_strings_are_understood() {
    for dir in versions() {
        let info = info(&dir);
        let s = &info["strings"];
        let text = |k: &str| s[k].as_str().unwrap_or_else(|| panic!("нет строки {k}"));

        // QR-код связи: адрес и код связи.
        assert_eq!(pairing::parse_qr(text("qr_invite")).unwrap(), invite());
        assert_eq!(pairing::normalize_code(text("code_shown")).unwrap(), invite().code);

        // Коннектор для Claude: доказательство ключа связи.
        assert_eq!(mcp::link_proof(&"ab".repeat(32), "polka", &"01".repeat(32)), text("link_proof"));

        // Имя файла событий в облаке.
        assert_eq!(cloud::event_name(DEVICE, 1, 500), text("event_name"));
        let parsed = cloud::parse_event_name(text("event_name")).unwrap();
        assert_eq!((parsed.author.as_str(), parsed.from, parsed.to), (DEVICE, 1, 500));
    }
}

fn invite() -> Invite {
    Invite { device: "a1b2c3".into(), name: "Ноутбук".into(), hosts: vec!["192.168.1.20".into(), "10.0.0.5".into()], port: 47_510, code: "6WVGHEWK".into() }
}

fn jpeg(n: usize, fill: u8) -> Vec<u8> {
    // Не одни и те же байты: так файл не сжимается и занимает в копии больше одного куска.
    let mut d: Vec<u8> = (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_be_bytes()[0] ^ fill).collect();
    d[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
    d
}

fn day(s: &str) -> Date {
    Date::parse(s).unwrap()
}

/// Делает образцы текущей версией: `cargo test -p polka-core --test formats -- --ignored make_fixtures`.
/// Готовую папку версии не перезаписывает.
#[test]
#[ignore = "создаёт образцы форматов этой версии; запускается руками"]
fn make_fixtures() {
    let version = env!("CARGO_PKG_VERSION");
    let out = fixtures().join(format!("v{version}"));
    assert!(!out.exists(), "образцы версии {version} уже есть — их не пересоздают");
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("polka.db");

    // Полка: книга с обложкой больше куска копии (1 МБ), фильм с маленькой обложкой, сериал с сезонами,
    // удалённая запись, цель на год, настройка, файл книги, место чтения и цитата.
    let mut store = Store::open(&db, &DB_KEY, DEVICE).unwrap();
    store.set_books_dir(tmp.path().join("books"));
    let big = store.put_blob("image/jpeg", &jpeg(1_100_000, 9)).unwrap();
    let small = store.put_blob("image/jpeg", &jpeg(40_000, 7)).unwrap();

    let mut book = Title::new(Kind::Book, "Пикник на обочине", day("2026-10-01"));
    book.id = "a".into();
    book.creators = vec!["Аркадий Стругацкий".into(), "Борис Стругацкий".into()];
    book.year = Some("1972".into());
    book.cover = Some(big);
    book.cover_source = Some("FantLab · АСТ, 2021".into());
    book.status = Status::Done;
    book.started = Some(day("2026-11-02"));
    book.finished = Some(day("2026-11-12"));
    book.rating = Some(9);
    book.note = "Перечитать через год.".into();
    book.tags = vec!["дача".into()];
    book.genres = vec!["Фантастика".into()];
    book.format = Some(Format::Paper);
    book.pages = Some(256);
    let mut film = Title::new(Kind::Film, "Сталкер", day("2026-11-05"));
    film.id = "b".into();
    film.creators = vec!["Андрей Тарковский".into()];
    film.year = Some("1979".into());
    film.cover = Some(small);
    film.minutes = Some(163);
    film.kp = Some(8.1);
    film.imdb = Some(8.0);
    let mut series = Title::new(Kind::Series, "Чернобыль", day("2026-11-06"));
    series.id = "c".into();
    series.status = Status::Doing;
    series.started = Some(day("2026-11-07"));
    series.seasons = vec![Season { no: 1, episodes: 5, seen: vec![Seen { ep: 1, on: day("2026-11-07") }, Seen { ep: 2, on: day("2026-11-08") }] }];
    for t in [&book, &film, &series] {
        t.check().unwrap();
        store.put(KIND, &t.id, t).unwrap();
    }
    let gone = Title { id: "gone".into(), ..Title::new(Kind::Book, "Удалённая", day("2026-11-01")) };
    store.put(KIND, "gone", &gone).unwrap();
    store.delete(KIND, "gone").unwrap();
    store.put(GOAL_KIND, "2026", &Goal { books: Some(24), screen: Some(12) }).unwrap();
    store.put(SETTING, "files.place", &"auto".to_string()).unwrap();

    // Файл книги — в папке книг устройства, запись о нём — в журнале.
    let pdf: Vec<u8> = b"%PDF-1.7\n".iter().copied().chain((0..300_000u32).map(|i| (i % 251) as u8)).collect();
    let book_id = blob_id(&pdf);
    fs::create_dir_all(store.books_dir().unwrap()).unwrap();
    fs::write(store.books_dir().unwrap().join(format!("{book_id}.pdf")), &pdf).unwrap();
    let rec = BookFile { id: book_id.clone(), title: "a".into(), name: "picnic.pdf".into(), format: BookFormat::Pdf, size: pdf.len() as u64, added: NOW - 3600 };
    rec.check().unwrap();
    store.put(FILE, &book_id, &rec).unwrap();
    let reading: Reading = serde_json::from_value(json!({ "title": "a", "book": book_id, "chapter": 3, "offset": 120, "percent": 41.5, "at": NOW - 600, "minutes": 95 })).unwrap();
    store.put(READING, "a", &reading).unwrap();
    let mark: Mark = serde_json::from_value(json!({ "id": "m1", "title": "a", "book": book_id, "kind": "quote", "chapter": 3, "offset": 40, "end": 90, "text": "Счастье для всех, даром", "note": "главное", "percent": 41.0, "created": NOW - 500 })).unwrap();
    mark.check().unwrap();
    store.put(MARK, "m1", &mark).unwrap();
    // Файлы книг — и в копию, и в облако.
    store.set_meta(BACKUP_KEY, "1").unwrap();
    store.set_meta(SYNC_KEY, "1").unwrap();

    let mut records = Map::new();
    for kind in KINDS {
        let ids: Vec<&str> = match kind {
            KIND => vec!["a", "b", "c"],
            GOAL_KIND => vec!["2026"],
            SETTING => vec!["files.place"],
            FILE => vec![book_id.as_str()],
            READING => vec!["a"],
            MARK => vec!["m1"],
            _ => unreachable!(),
        };
        let bodies: Map<String, Value> = ids.into_iter().map(|id| (id.to_string(), store.get::<Value>(kind, id).unwrap().unwrap())).collect();
        records.insert(kind.to_string(), Value::Object(bodies));
    }
    let blobs: Map<String, Value> = store.present_blobs().unwrap().into_iter().map(|id| {
        let size = store.blob(&id).unwrap().unwrap().1.len();
        (id, json!(size))
    }).collect();

    // Резервная копия — с обычными параметрами ключа, как у людей.
    let backup_key = BackupKey::new(PASSWORD).unwrap();
    let by = MadeBy { app: version.into(), created: NOW, device: "Ноутбук".into(), device_id: DEVICE.into() };
    let mut backup_file = Vec::new();
    let made = backup::write_store(&store, &backup_key, &by, &mut backup_file).unwrap();
    assert_eq!(made.blobs, 2);

    // Облако: ключ из пароля и один обмен.
    let cloud = MemCloud::default();
    let (cloud_key, created) = cloud::join(&cloud, PASSWORD, Kdf::DEFAULT, NOW).unwrap();
    assert!(created);
    let versions = store.versions().unwrap();
    let events = store.event_count().unwrap();
    let store = Mutex::new(store);
    let cancel = AtomicBool::new(false);
    let report = cloud::sync(&Round { cloud: &cloud, store: &store, key: &cloud_key, now: NOW, cancel: &cancel }, &mut State::default(), &mut |_| {}).unwrap();
    assert_eq!(report.books_sent, 1);
    // База закрыта — журнал записи (WAL) слит в файл.
    drop(store);

    fs::create_dir_all(&out).unwrap();
    fs::copy(&db, out.join("store.db")).unwrap();
    fs::write(out.join("backup.polka"), &backup_file).unwrap();
    for d in DIRS {
        let folder = out.join("cloud").join(d.folder());
        fs::create_dir_all(&folder).unwrap();
        for name in cloud.names(d) {
            fs::write(folder.join(&name), cloud.raw(d, &name).unwrap()).unwrap();
        }
    }
    let names: Vec<Vec<String>> = DIRS.iter().map(|d| cloud.names(*d)).collect();
    let info = json!({
        "version": version,
        "made": "cargo test -p polka-core --test formats -- --ignored make_fixtures",
        "db_key": hex(&DB_KEY),
        "device": DEVICE,
        "password": PASSWORD,
        "records": records,
        "blobs": blobs,
        "book": { "id": book_id, "size": pdf.len() },
        "versions": versions,
        "events": events,
        "cloud_files": names,
        "strings": {
            "qr_invite": invite().to_uri(),
            "code_shown": pairing::show_code(&invite().code),
            "backup_key": backup_key.to_meta(),
            "cloud_key": cloud_key.to_meta(),
            "cloud_group": cloud_key.group(),
            "link_proof": mcp::link_proof(&"ab".repeat(32), "polka", &"01".repeat(32)),
            "event_name": cloud::event_name(DEVICE, 1, 500),
        },
    });
    fs::write(out.join("info.json"), serde_json::to_string_pretty(&info).unwrap() + "\n").unwrap();
}

//! Облако: Яндекс Диск или свой сервер WebDAV — общий ящик для устройств одного человека.
//!
//! С 0.11.8 формат облака и обмен с ним — общие, из kit (`kit_core::cloud`), те же, что у «Кармана».
//! Здесь — то, чем «Полка» отличается ([`FORMAT`]): файл ключа `polka.key`, метка файлов `PLKC`,
//! расширения `.pev`, `.pbl` и `.pbk`, обложки вместо снимков чеков и файлы книг кусками по 8 МБ.
//! Менять их нельзя: устройства перестанут читать то, что уже лежит в облаке.
//!
//! - `polka.key` — ключ облака, зашифрованный Argon2id от пароля копий.
//! - `e/<устройство>-<с>-<по>.pev` — события одного устройства, сжатые и зашифрованные.
//! - `b/<имя>.pbl` — обложка; имя — MAC от отпечатка снимка.
//! - `k/<имя>-<кусок>-<всего>.pbk` — файл книги кусками (с 0.8.0, если включён обмен файлами книг).

use crate::backup::Kdf;
use crate::db::{self, Event, Store};
use crate::error::{Error, Result};
use crate::files;
use kit_core::cloud::{self as kit, CloudStore};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

pub use kit::{Cloud, Dir, Entry, EventFileName, KeyFile, MemCloud, State, COMPACT_AFTER, FILE_EVENTS, MAX_EVENTS_FILE, MAX_KEY_FILE};

/// Файл ключа облака в папке «Полки».
pub const KEY_FILE: &str = "polka.key";
/// Предел файла обложки: снимок и немного на заголовок.
pub const MAX_BLOB_FILE: usize = db::MAX_BLOB + 1024;
/// Предел куска файла книги в облаке.
pub const MAX_BOOK_FILE: usize = files::PART + kit::OVERHEAD + 64;

fn cover_ok(mime: &str, data: &[u8]) -> bool {
    db::check_blob(mime, data).is_ok()
}

/// Облако «Полки». У «Кармана» — `karman.key`, `KRMC`, `.kev`, `.kbl` и снимки чеков.
pub static FORMAT: kit::Format = kit::Format {
    names: crate::NAMES,
    key_file: KEY_FILE,
    magic: *b"PLKC",
    events_ext: ".pev",
    // Номера событий в базе «Полки» — i64.
    max_seq: i64::MAX as u64,
    blobs: kit::Blobs { ext: ".pbl", max_file: MAX_BLOB_FILE, what: "обложка", this: "эта обложка", of: "обложки", feminine: true, check: cover_ok },
    files: Some(kit::Files { ext: ".pbk", part: files::PART, what: "Файл книги" }),
};

fn store_err(e: Error) -> kit_core::Error {
    kit_core::Error::Store(e.to_string())
}

/// Чего обмену с облаком нужно от базы «Полки» сверх обмена по Wi-Fi.
impl CloudStore for Store {
    fn origin(e: &Event) -> (&str, u64) {
        (&e.device, e.seq.max(0) as u64)
    }
    fn events_range(&self, author: &str, from: u64, to: u64) -> kit_core::Result<Vec<Event>> {
        Store::events_range(self, author, from as i64, to as i64).map_err(store_err)
    }
    fn differs(&self, e: &Event) -> kit_core::Result<bool> {
        Ok(Store::event_body(self, &e.device, e.seq).map_err(store_err)?.is_some_and(|b| b != e.body))
    }
    fn present_blobs(&self) -> kit_core::Result<Vec<String>> {
        Store::present_blobs(self).map_err(store_err)
    }
}

/// Ключ облака. Хранится в базе устройства (она сама зашифрована) — для обмена пароль не нужен.
pub struct CloudKey(kit::CloudKey);

impl CloudKey {
    /// Новый ключ для облака, которое подключают впервые.
    pub fn generate() -> Result<CloudKey> {
        Ok(CloudKey(kit::CloudKey::generate(&FORMAT)?))
    }

    pub fn group(&self) -> &str {
        self.0.group()
    }

    /// Для хранения в базе: «1:номер облака:ключ».
    pub fn to_meta(&self) -> String {
        self.0.to_meta()
    }

    pub fn from_meta(s: &str) -> Result<CloudKey> {
        Ok(CloudKey(kit::CloudKey::from_meta(&FORMAT, s)?))
    }

    /// Файл ключа для облака: ключ облака, зашифрованный паролем копий (около секунды).
    pub fn key_file(&self, password: &str, kdf: Kdf, changed: i64) -> Result<KeyFile> {
        Ok(self.0.key_file(password, kdf, changed)?)
    }

    /// Ключ облака из файла ключа по паролю копий (около секунды).
    pub fn from_key_file(kf: &KeyFile, password: &str) -> Result<CloudKey> {
        Ok(CloudKey(kit::CloudKey::from_key_file(&FORMAT, kf, password)?))
    }

    /// Имя файла обложки в облаке: по нему не узнать отпечаток снимка.
    pub fn blob_name(&self, id: &str) -> String {
        self.0.blob_name(id)
    }

    /// Имя куска файла книги в облаке: по нему не узнать отпечаток файла.
    pub fn book_name(&self, id: &str, part: u64, parts: u64) -> String {
        self.0.file_name(id, part, parts)
    }
}

/// Файл ключа из облака.
pub fn parse_key_file(data: &[u8]) -> Result<KeyFile> {
    Ok(KeyFile::parse(&FORMAT, data)?)
}

pub fn read_key_file(cloud: &dyn Cloud) -> Result<Option<KeyFile>> {
    Ok(kit::read_key_file(&FORMAT, cloud)?)
}

/// Подключает устройство по паролю копий: ключ облака уже есть — открывает его, нет — создаёт.
/// Возвращает ключ и `true`, если облако подключено впервые.
pub fn join(cloud: &dyn Cloud, password: &str, kdf: Kdf, now: i64) -> Result<(CloudKey, bool)> {
    let (key, created) = kit::join(&FORMAT, cloud, password, kdf, now)?;
    Ok((CloudKey(key), created))
}

/// Новый пароль копий: тот же ключ облака, зашифрованный новым паролем.
pub fn put_key_file(cloud: &dyn Cloud, key: &CloudKey, kf: &KeyFile) -> Result<()> {
    Ok(kit::put_key_file(cloud, &key.0, kf)?)
}

/// Удаляет данные «Полки» из облака. Сначала ключ: другие устройства увидят это и перестанут слать файлы.
pub fn wipe(cloud: &dyn Cloud, cancel: &AtomicBool) -> Result<usize> {
    Ok(kit::wipe(&FORMAT, cloud, cancel)?)
}

/// Имя файла событий: устройство и номера событий «с…по».
pub fn event_name(author: &str, from: u64, to: u64) -> String {
    kit::event_name(&FORMAT, author, from, to)
}

/// Разбирает имя файла событий. Чужие и неканонические имена («007» вместо «7») не подходят.
pub fn parse_event_name(name: &str) -> Option<EventFileName> {
    kit::parse_event_name(&FORMAT, name)
}

/// Один обмен с облаком.
pub struct Round<'a> {
    pub cloud: &'a dyn Cloud,
    pub store: &'a Mutex<Store>,
    pub key: &'a CloudKey,
    /// Сейчас: секунды Unix.
    pub now: i64,
    /// Остановить обмен между запросами.
    pub cancel: &'a AtomicBool,
}

/// Чем занят обмен — для экрана.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Events,
    Upload { done: usize, total: usize },
    Download { done: usize, total: usize },
    /// Файлы книг: отправка и получение.
    Books { done: usize, total: usize },
}

impl From<kit::Step> for Step {
    fn from(s: kit::Step) -> Self {
        match s {
            kit::Step::Events => Step::Events,
            kit::Step::Upload { done, total } => Step::Upload { done, total },
            kit::Step::Download { done, total } => Step::Download { done, total },
            kit::Step::Files { done, total } => Step::Books { done, total },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Новых событий пришло.
    pub received: usize,
    /// Событий отправлено в облако.
    pub sent: usize,
    /// Обложек пришло и ушло.
    pub files_received: usize,
    pub files_sent: usize,
    /// Обложек, которых в облаке пока нет: их ещё не отправило другое устройство.
    pub files_waiting: usize,
    /// Файлов книг пришло и ушло; сколько ещё не лежит в облаке целиком.
    #[serde(default)]
    pub books_received: usize,
    #[serde(default)]
    pub books_sent: usize,
    #[serde(default)]
    pub books_waiting: usize,
    /// Что не получилось, но обмену не помешало.
    pub problems: Vec<String>,
}

impl From<kit::Report> for Report {
    fn from(r: kit::Report) -> Self {
        Report {
            received: r.received,
            sent: r.sent,
            files_received: r.files_received,
            files_sent: r.files_sent,
            files_waiting: r.files_waiting,
            books_received: r.whole_received,
            books_sent: r.whole_sent,
            books_waiting: r.whole_waiting,
            problems: r.problems,
        }
    }
}

/// Обмен с облаком: забрать новое, отправить недостающее, слить мелкие файлы, обложки, файлы книг.
pub fn sync(r: &Round, state: &mut State, progress: &mut dyn FnMut(Step)) -> Result<Report> {
    let round = kit::Round { cloud: r.cloud, store: r.store, key: &r.key.0, now: r.now, cancel: r.cancel };
    Ok(kit::sync(&round, state, &mut |s| progress(s.into()))?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Title, KIND};
    use crate::error::CloudFail;
    use std::collections::BTreeMap;
    use std::sync::MutexGuard;

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
    use crate::Date;
    use std::path::Path;

    const FAST: Kdf = Kdf { m_kib: 8 * 1024, t: 1, p: 1 };
    const PW: &str = "отпуск в Сочи";

    fn jpeg(n: usize, fill: u8) -> Vec<u8> {
        let mut d = vec![fill; n];
        d[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        d
    }

    fn title(id: &str, name: &str) -> Title {
        let mut t = Title::new(Kind::Book, name, Date::parse("2026-11-13").unwrap());
        t.id = id.into();
        t
    }

    /// База с тремя книгами; у первой — обложка.
    fn demo(dir: &Path, name: &str, device: &str) -> Mutex<Store> {
        let mut s = Store::open(&dir.join(name), &[5u8; 32], device).unwrap();
        let cover = s.put_blob("image/jpeg", &jpeg(50_000, 7)).unwrap();
        for (i, n) in ["Пикник на обочине", "Лавр", "Щегол"].into_iter().enumerate() {
            let mut t = title(&format!("{device}{i}"), n);
            if i == 0 {
                t.cover = Some(cover.clone());
            }
            s.put(KIND, &t.id, &t).unwrap();
        }
        Mutex::new(s)
    }

    fn fresh(dir: &Path, name: &str, device: &str) -> Mutex<Store> {
        Mutex::new(Store::open(&dir.join(name), &[6u8; 32], device).unwrap())
    }

    fn set(s: &Mutex<Store>, key: &str, value: &str) {
        lock(s).put("setting", key, &value.to_string()).unwrap();
    }

    fn get(s: &Mutex<Store>, key: &str) -> Option<String> {
        lock(s).get::<String>("setting", key).unwrap()
    }

    fn round(cloud: &dyn Cloud, store: &Mutex<Store>, key: &CloudKey, state: &mut State) -> Result<Report> {
        let cancel = AtomicBool::new(false);
        sync(&Round { cloud, store, key, now: 1_794_600_000, cancel: &cancel }, state, &mut |_| {})
    }

    fn titles(s: &Mutex<Store>) -> serde_json::Value {
        let mut list: Vec<Title> = lock(s).list(KIND).unwrap();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        serde_json::to_value(list).unwrap()
    }

    fn same_data(a: &Mutex<Store>, b: &Mutex<Store>) {
        assert_eq!(titles(a), titles(b));
        assert_eq!(lock(a).versions().unwrap(), lock(b).versions().unwrap());
    }

    fn versions(s: &Mutex<Store>) -> BTreeMap<String, i64> {
        lock(s).versions().unwrap()
    }

    #[test]
    fn names_are_canonical() {
        assert_eq!(parse_event_name("0a1b-1-500.pev"), Some(EventFileName { author: "0a1b".into(), from: 1, to: 500 }));
        for bad in ["0a1b-01-500.pev", "0a1b-0-5.pev", "0a1b-9-5.pev", "0a1b-1-5.pev.tmp", "a-b-1-5.pev", "-1-5.pev", "x/y-1-5.pev", "0a1b-1-5.kev", "polka.key", "0a1b-1-9223372036854775808.pev"] {
            assert_eq!(parse_event_name(bad), None, "{bad}");
        }
        let mk = CloudKey::generate().unwrap();
        let id = db::blob_id(b"x");
        let name = mk.blob_name(&id);
        assert_eq!(name.len(), 32 + 4);
        assert!(name.ends_with(".pbl"));
        assert!(!name.contains(&id[..16]), "по имени не узнать отпечаток обложки");
        assert_eq!(name, mk.blob_name(&id));
        assert_ne!(name, CloudKey::generate().unwrap().blob_name(&id), "у другого облака другие имена");
    }

    /// Файлы книг через облако: кусками, только где включён обмен файлами, с ожиданием недостающих кусков.
    #[test]
    fn book_files_go_through_the_cloud_in_parts() {
        use crate::files::{BookFile, FILE, PART, SYNC_KEY};
        use crate::reader::BookFormat;
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let (key, _) = join(&cloud, PW, FAST, 1).unwrap();
        let a = demo(dir.path(), "a.db", "aaaa");
        let b = fresh(dir.path(), "b.db", "bbbb");
        lock(&a).set_books_dir(dir.path().join("a-books"));
        lock(&b).set_books_dir(dir.path().join("b-books"));
        let data: Vec<u8> = b"%PDF-1.7\n".iter().copied().chain((0..(PART * 2 + 7)).map(|i| (i % 241) as u8)).collect();
        let id = db::blob_id(&data);
        {
            let mut st = lock(&a);
            let d = st.books_dir().unwrap().to_path_buf();
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{id}.pdf")), &data).unwrap();
            st.put(FILE, &id, &BookFile { id: id.clone(), title: "aaaa0".into(), name: "picnic.pdf".into(), format: BookFormat::Pdf, size: data.len() as u64, added: 1 }).unwrap();
        }
        let (mut sa, mut sb) = (State::default(), State::default());
        // Обмен файлами выключен: в облаке только записи.
        round(&cloud, &a, &key, &mut sa).unwrap();
        assert!(cloud.names(Dir::Files).is_empty());
        lock(&a).set_meta(SYNC_KEY, "1").unwrap();
        let r = round(&cloud, &a, &key, &mut sa).unwrap();
        assert_eq!(r.books_sent, 1);
        let names = cloud.names(Dir::Files);
        assert_eq!(names.len(), 3, "три куска: {names:?}");
        assert!(names.iter().all(|n| n.ends_with(".pbk") && !n.contains(&id[..16])));
        // Второе устройство: записи пришли, файла нет, пока обмен файлами выключен.
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!(r.books_received, 0);
        lock(&b).set_meta(SYNC_KEY, "1").unwrap();
        // Пока в облаке не все куски — ждём.
        let last = key.book_name(&id, 2, 3);
        let kept = cloud.raw(Dir::Files, &last).unwrap();
        cloud.remove(Dir::Files, &last);
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_waiting), (0, 1));
        // Подменённый кусок не пройдёт: имя входит в проверку подлинности.
        cloud.set_raw(Dir::Files, &last, cloud.raw(Dir::Files, &key.book_name(&id, 1, 3)).unwrap());
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!(r.books_received, 0);
        assert!(!r.problems.is_empty());
        cloud.set_raw(Dir::Files, &last, kept);
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_waiting), (1, 0), "{r:?}");
        let got = crate::files::path_in(lock(&b).books_dir().unwrap(), &id).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), data);
        // Больше делать нечего.
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_sent), (0, 0));
        // Удаление данных из облака убирает и куски файлов.
        wipe(&cloud, &AtomicBool::new(false)).unwrap();
        assert!(cloud.names(Dir::Files).is_empty());
    }

    #[test]
    fn two_devices_exchange_titles_and_covers() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = demo(dir.path(), "a.db", "aaaa");
        let b = fresh(dir.path(), "b.db", "bbbb");
        let (ka, created) = join(&cloud, PW, FAST, 1).unwrap();
        assert!(created);
        assert!(matches!(join(&cloud, "чужой пароль", FAST, 1), Err(Error::CloudPassword)));
        let (kb, created) = join(&cloud, PW, FAST, 2).unwrap();
        assert!(!created);
        assert_eq!(ka.group(), kb.group());

        let (mut sa, mut sb) = (State::default(), State::default());
        let r = round(&cloud, &a, &ka, &mut sa).unwrap();
        let total = lock(&a).event_count().unwrap() as usize;
        assert_eq!((r.received, r.sent, r.files_sent), (0, total, 1));
        let r = round(&cloud, &b, &kb, &mut sb).unwrap();
        assert_eq!((r.received, r.sent, r.files_received, r.files_waiting), (total, 0, 1, 0));
        let r = round(&cloud, &a, &ka, &mut sa).unwrap();
        assert_eq!((r.received, r.sent), (0, 0));
        same_data(&a, &b);
        assert!(lock(&b).missing_blobs().unwrap().is_empty(), "обложка пришла");

        // Запись на одном устройстве появляется на другом.
        let mut t: Title = lock(&b).get(KIND, "aaaa1").unwrap().unwrap();
        t.rating = Some(9);
        lock(&b).put(KIND, &t.id, &t).unwrap();
        assert_eq!(round(&cloud, &b, &kb, &mut sb).unwrap().sent, 1);
        assert_eq!(round(&cloud, &a, &ka, &mut sa).unwrap().received, 1);
        assert_eq!(lock(&a).get::<Title>(KIND, "aaaa1").unwrap().unwrap().rating, Some(9));
        // Без изменений обмен ничего не гоняет.
        let r = round(&cloud, &a, &ka, &mut sa).unwrap();
        assert_eq!((r.received, r.sent, r.files_sent, r.files_received), (0, 0, 0, 0));
        same_data(&a, &b);
    }

    #[test]
    fn records_from_wifi_go_through_the_cloud_and_are_not_sent_twice() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = demo(dir.path(), "a.db", "aaaa");
        let b = fresh(dir.path(), "b.db", "bbbb");
        let c = fresh(dir.path(), "c.db", "cccc");
        lock(&c).put(KIND, "c1", &title("c1", "Брат")).unwrap();
        // C связан с B только по Wi-Fi.
        let to_b = lock(&c).events_missing(&versions(&b)).unwrap();
        lock(&b).apply_remote(&to_b).unwrap();
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let (mut sa, mut sb, mut sc) = (State::default(), State::default(), State::default());
        round(&cloud, &a, &k, &mut sa).unwrap();
        round(&cloud, &b, &k, &mut sb).unwrap();
        round(&cloud, &a, &k, &mut sa).unwrap();
        assert!(versions(&a).contains_key("cccc"), "записи C пришли через B");
        // Потом C подключает облако сам: своё он уже не отправляет.
        let r = round(&cloud, &c, &k, &mut sc).unwrap();
        assert_eq!(r.sent, 0);
        same_data(&a, &c);
    }

    #[test]
    fn small_files_are_merged_and_a_new_device_gets_everything() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = fresh(dir.path(), "a.db", "aaaa");
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let mut sa = State::default();
        for i in 0..20 {
            set(&a, &format!("k{i}"), &i.to_string());
            round(&cloud, &a, &k, &mut sa).unwrap();
        }
        let names = cloud.names(Dir::Events);
        assert!(names.len() <= COMPACT_AFTER + 1, "мелкие файлы слиты: {names:?}");
        assert!(names.iter().any(|n| n.starts_with("aaaa-1-")), "{names:?}");
        let b = fresh(dir.path(), "b.db", "bbbb");
        let mut sb = State::default();
        round(&cloud, &b, &k, &mut sb).unwrap();
        assert_eq!(versions(&b).get("aaaa"), Some(&20));
        assert_eq!(get(&b, "k19").as_deref(), Some("19"));
    }

    #[test]
    fn a_gap_in_the_cloud_is_not_applied_and_heals() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = fresh(dir.path(), "a.db", "aaaa");
        for i in 0..3 {
            set(&a, &format!("s{i}"), "1");
        }
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let mut sa = State::default();
        round(&cloud, &a, &k, &mut sa).unwrap(); // aaaa-1-3
        for i in 0..2 {
            set(&a, &format!("g{i}"), "1");
            round(&cloud, &a, &k, &mut sa).unwrap(); // aaaa-4-4, aaaa-5-5
        }
        cloud.remove(Dir::Events, &event_name("aaaa", 4, 4));
        let b = fresh(dir.path(), "b.db", "bbbb");
        let mut sb = State::default();
        round(&cloud, &b, &k, &mut sb).unwrap();
        assert_eq!(versions(&b).get("aaaa"), Some(&3), "после пропуска ничего не применяется");
        // Устройство A видит пропуск и отправляет недостающее снова.
        assert_eq!(round(&cloud, &a, &k, &mut sa).unwrap().sent, 1);
        round(&cloud, &b, &k, &mut sb).unwrap();
        assert_eq!(versions(&b).get("aaaa"), Some(&5));
    }

    #[test]
    fn deleted_or_reconnected_cloud_stops_the_exchange() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = fresh(dir.path(), "a.db", "aaaa");
        for i in 0..3 {
            set(&a, &format!("s{i}"), "1");
        }
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let mut sa = State::default();
        round(&cloud, &a, &k, &mut sa).unwrap();
        // Облако подключили заново на другом устройстве — с новым ключом.
        let cancel = AtomicBool::new(false);
        wipe(&cloud, &cancel).unwrap();
        assert!(cloud.names(Dir::Events).is_empty() && cloud.raw(Dir::Root, KEY_FILE).is_none());
        assert!(matches!(round(&cloud, &a, &k, &mut sa), Err(Error::Cloud(CloudFail::Gone, _))));
        let (k2, created) = join(&cloud, PW, FAST, 2).unwrap();
        assert!(created);
        assert!(matches!(round(&cloud, &a, &k, &mut sa), Err(Error::Cloud(CloudFail::Replaced, _))));
        // По паролю устройство получает новый ключ и снова обменивается.
        let (k3, created) = join(&cloud, PW, FAST, 3).unwrap();
        assert!(!created);
        assert_eq!(k3.group(), k2.group());
        let mut fresh_state = State::default();
        assert_eq!(round(&cloud, &a, &k3, &mut fresh_state).unwrap().sent, 3);
        // Новый пароль копий: ключ облака тот же, открывается новым паролем.
        let kf = k3.key_file("новый пароль копий", FAST, 4).unwrap();
        put_key_file(&cloud, &k3, &kf).unwrap();
        assert!(matches!(join(&cloud, PW, FAST, 5), Err(Error::CloudPassword)));
        let (k4, _) = join(&cloud, "новый пароль копий", FAST, 5).unwrap();
        assert_eq!(k4.to_meta(), k3.to_meta());
        assert!(matches!(put_key_file(&cloud, &k, &k.key_file(PW, FAST, 6).unwrap()), Err(Error::Cloud(CloudFail::Replaced, _))));
    }

    #[test]
    fn only_polka_files_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = demo(dir.path(), "a.db", "aaaa");
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        round(&cloud, &a, &k, &mut State::default()).unwrap();
        // Свои файлы человека в папках «Полки».
        for (d, n) in [(Dir::Events, "заметки.txt"), (Dir::Blobs, "фото.jpg"), (Dir::Events, "aaaa-01-3.pev"), (Dir::Root, "список.docx")] {
            cloud.set_raw(d, n, b"my file".to_vec());
        }
        let cancel = AtomicBool::new(false);
        wipe(&cloud, &cancel).unwrap();
        assert_eq!(cloud.names(Dir::Events), ["aaaa-01-3.pev", "заметки.txt"]);
        assert_eq!(cloud.names(Dir::Blobs), ["фото.jpg"]);
        assert_eq!(cloud.names(Dir::Root), ["список.docx"]);
        // Подключение заново тоже убирает только файлы «Полки».
        round(&cloud, &a, &join(&cloud, PW, FAST, 2).unwrap().0, &mut State::default()).unwrap();
        cloud.remove(Dir::Root, KEY_FILE);
        join(&cloud, PW, FAST, 3).unwrap();
        assert_eq!(cloud.names(Dir::Events), ["aaaa-01-3.pev", "заметки.txt"]);
        assert_eq!(cloud.names(Dir::Blobs), ["фото.jpg"]);
    }

    #[test]
    fn same_device_number_with_other_records_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = fresh(dir.path(), "a.db", "aaaa");
        set(&a, "goal", "24");
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let mut sa = State::default();
        round(&cloud, &a, &k, &mut sa).unwrap();
        // База начата заново под тем же номером устройства и с другой записью.
        let again = fresh(dir.path(), "a2.db", "aaaa");
        set(&again, "goal", "52");
        let mut s2 = State::default();
        assert!(matches!(round(&cloud, &again, &k, &mut s2), Err(Error::Cloud(CloudFail::Conflict, _))));
        assert_eq!(get(&again, "goal").as_deref(), Some("52"), "база не тронута");
        // Та же база, что отправляла, расхождений не находит.
        let mut s3 = State::default();
        assert!(round(&cloud, &a, &k, &mut s3).is_ok());
    }

    #[test]
    fn cancel_stops_between_requests() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = demo(dir.path(), "a.db", "aaaa");
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let cancel = AtomicBool::new(true);
        let mut st = State::default();
        let r = sync(&Round { cloud: &cloud, store: &a, key: &k, now: 1, cancel: &cancel }, &mut st, &mut |_| {});
        assert!(matches!(r, Err(Error::Cloud(CloudFail::Cancelled, _))));
        assert!(cloud.names(Dir::Events).is_empty());
    }
}

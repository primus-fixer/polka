//! Резервная копия «Полки»: весь журнал событий, обложки и — если так выбрано — файлы книг в одном файле,
//! зашифрованном паролем копии.
//!
//! Формат файла — общий из kit (`kit_core::backup`): заголовок со своей меткой `POLKABAK`, ключ копий
//! (Argon2id от пароля), шифрование кусками по 1 МБ, сведения о копии, пачки событий, вложения и итог.
//! Своё у «Полки» — запись 4: кусок файла книги (с 0.8.0; кусками по 8 МБ). Версии до 0.8.0 её пропускают.
//! Восстановление добавляет события к базе так же, как обмен: повторы пропускаются, при споре побеждает
//! более позднее изменение.

use crate::db::{Event, Store};
use crate::error::Result;
use crate::files::{self, BookFile};
use crate::model::{Title, KIND};
use kit_core::backup::{self as kit, Format};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

pub use kit::{check_password, derive_raw, random, wipe, BackupKey, Kdf, MadeBy, FORMAT, MIN_PASSWORD};

/// Метка в начале файла копии. **Менять нельзя:** старые копии перестанут узнаваться.
pub const MAGIC: &[u8; 8] = b"POLKABAK";
/// Расширение файла копии.
pub const EXT: &str = "polka";
/// Кусок файла книги (с 0.8.0).
const REC_BOOK: u8 = 4;

/// Копии «Полки» для формата kit.
pub const BACKUP: Format = Format { magic: MAGIC, ext: EXT, names: crate::NAMES, common: &["polka", "полка"], records: &[(REC_BOOK, "books")] };

/// Новый пароль копий: не короче восьми символов и не из самых простых (в том числе не «полка»).
pub fn check_new_password(password: &str) -> Result<()> {
    Ok(kit::check_new_password(&BACKUP, password)?)
}

/// Самые простые пароли: возвращает, чем плох пароль.
pub fn weak_password(password: &str) -> Option<&'static str> {
    kit::weak_password(&BACKUP, password)
}

/// Файл начинается как копия «Полки».
pub fn is_backup(path: &Path) -> bool {
    kit::is_backup(&BACKUP, path)
}

/// Счётчики «Полки» в сведениях о копии.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    /// Записей о книгах, фильмах и сериалах.
    pub titles: u64,
    /// Файлов книг в копии (с 0.8.0).
    #[serde(default)]
    pub books: u64,
}

/// Сведения о копии — первая запись. По ним видно, что восстанавливается.
pub type Manifest = kit::Manifest<Counts>;

/// Сведения о копии «Полки»: сколько событий, обложек, записей и файлов книг.
pub fn manifest(by: &MadeBy, events: u64, blobs: u64, titles: u64, books: u64) -> Manifest {
    Manifest::new(by, events, blobs, Counts { titles, books })
}

/// Заголовок куска файла книги в копии.
#[derive(Serialize, Deserialize)]
struct BookPart {
    file: BookFile,
    offset: u64,
}

/// Что записано.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Made {
    pub events: u64,
    pub blobs: u64,
    /// Файлов книг.
    pub books: u64,
    pub bytes: u64,
}

/// Файл копии пишется по частям: события, затем обложки и куски книг по одному — в памяти не больше куска.
pub struct Writer<W: Write> {
    inner: kit::Writer<W>,
    books: u64,
}

impl<W: Write> Writer<W> {
    pub fn new(out: W, key: &BackupKey, manifest: &Manifest) -> Result<Writer<W>> {
        Ok(Writer { inner: kit::Writer::new(&BACKUP, out, key, manifest)?, books: 0 })
    }

    pub fn events(&mut self, events: &[Event]) -> Result<()> {
        Ok(self.inner.events(events)?)
    }

    pub fn blob(&mut self, id: &str, mime: &str, data: &[u8]) -> Result<()> {
        Ok(self.inner.blob(id, mime, data)?)
    }

    /// Кусок файла книги: заголовок (длина и JSON) и данные.
    pub fn book_part(&mut self, file: &BookFile, offset: u64, data: &[u8]) -> Result<()> {
        let head = serde_json::to_vec(&BookPart { file: file.clone(), offset })?;
        let mut p = Vec::with_capacity(4 + head.len() + data.len());
        p.extend_from_slice(&(head.len() as u32).to_le_bytes());
        p.extend_from_slice(&head);
        p.extend_from_slice(data);
        self.inner.custom(REC_BOOK, &p)?;
        if offset + data.len() as u64 >= file.size {
            self.books += 1;
        }
        Ok(())
    }

    /// Итог и последний кусок. Возвращает, сколько записано, и куда писали.
    pub fn finish(self) -> Result<(Made, W)> {
        let (m, out) = self.inner.finish()?;
        Ok((Made { events: m.events, blobs: m.blobs, books: self.books, bytes: m.bytes }, out))
    }
}

/// Что встретилось в копии.
pub enum Item {
    Events(Vec<Event>),
    Blob { id: String, mime: String, data: Vec<u8> },
    /// Кусок файла книги с места `offset`.
    Book { file: BookFile, offset: u64, data: Vec<u8> },
}

/// Чтение копии: ключ проверяется сразу, куски — по мере чтения.
pub struct Reader<R: Read> {
    inner: kit::Reader<R, Counts>,
    pub manifest: Manifest,
    pub events: u64,
    pub blobs: u64,
    /// Файлов книг (последний кусок прочитан).
    pub books: u64,
}

impl<R: Read> Reader<R> {
    /// Открывает копию паролем (ключ считается заново — около секунды).
    pub fn open(input: R, password: &str) -> Result<Reader<R>> {
        Ok(Reader::wrap(kit::Reader::open(&BACKUP, input, password)?))
    }

    /// Открывает копию сохранённым ключом — проверка только что записанной копии.
    pub fn open_with_key(input: R, key: &BackupKey) -> Result<Reader<R>> {
        Ok(Reader::wrap(kit::Reader::open_with_key(&BACKUP, input, key)?))
    }

    fn wrap(inner: kit::Reader<R, Counts>) -> Reader<R> {
        let manifest = inner.manifest.clone();
        Reader { inner, manifest, events: 0, blobs: 0, books: 0 }
    }

    /// Следующая запись: пачка событий, обложка или кусок книги. `None` — копия прочитана целиком и сошлась.
    pub fn next_item(&mut self) -> Result<Option<Item>> {
        let item = self.inner.next_item::<Event>()?;
        (self.events, self.blobs) = (self.inner.events, self.inner.blobs);
        Ok(match item {
            None => None,
            Some(kit::Item::Events(events)) => Some(Item::Events(events)),
            Some(kit::Item::Blob { id, mime, data }) => Some(Item::Blob { id, mime, data }),
            Some(kit::Item::Custom { payload, .. }) => {
                let bad = || crate::error::Error::Backup("Копия повреждена: файл книги не читается".into());
                let len = u32::from_le_bytes(payload.get(..4).ok_or_else(bad)?.try_into().map_err(|_| bad())?) as usize;
                let head: BookPart = serde_json::from_slice(payload.get(4..4 + len).ok_or_else(bad)?).map_err(|_| bad())?;
                let data = payload[4 + len..].to_vec();
                if head.offset + data.len() as u64 >= head.file.size {
                    self.books += 1;
                }
                Some(Item::Book { file: head.file, offset: head.offset, data })
            }
        })
    }
}

/// Итог проверки: копия прочитана целиком.
#[derive(Clone, Debug, Serialize)]
pub struct Checked {
    pub manifest: Manifest,
    pub events: u64,
    pub blobs: u64,
    /// Файлов книг в копии.
    pub books: u64,
    /// События этого же устройства, которые расходятся с базой (копия с устройства,
    /// база которого была начата заново под тем же номером).
    pub conflicts: u64,
}

/// Читает копию целиком, ничего не меняя. `conflict` получает события этого устройства
/// и говорит, расходятся ли они с базой.
pub fn verify<R: Read>(mut r: Reader<R>, device: &str, mut conflict: impl FnMut(&Event) -> Result<bool>) -> Result<Checked> {
    let mut conflicts = 0;
    while let Some(item) = r.next_item()? {
        if let Item::Events(events) = item {
            for ev in events.iter().filter(|e| e.device == device) {
                if conflict(ev)? {
                    conflicts += 1;
                }
            }
        }
    }
    Ok(Checked { manifest: r.manifest.clone(), events: r.events, blobs: r.blobs, books: r.books, conflicts })
}

/// Событие этого устройства расходится с базой: под тем же номером записано другое.
pub fn conflicts_with(store: &Store, ev: &Event) -> Result<bool> {
    Ok(store.event_body(&ev.device, ev.seq)?.is_some_and(|b| b != ev.body))
}

/// Сколько добавилось при восстановлении.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Restored {
    pub events: u64,
    pub blobs: u64,
    /// Файлов книг легло на устройство.
    pub books: u64,
}

/// Копия всей базы: события, все обложки и — если так выбрано (`files.backup`) — файлы книг.
pub fn write_store<W: Write>(store: &Store, key: &BackupKey, by: &MadeBy, out: W) -> Result<Made> {
    let events = store.events_missing(&Default::default())?;
    let blobs = store.present_blobs()?;
    let books = if files::backup_on(store) { files::present(store)? } else { Vec::new() };
    let manifest = manifest(by, events.len() as u64, blobs.len() as u64, store.list::<Title>(KIND)?.len() as u64, books.len() as u64);
    let mut w = Writer::new(out, key, &manifest)?;
    w.events(&events)?;
    for id in &blobs {
        if let Some((mime, data)) = store.blob(id)? {
            w.blob(id, &mime, &data)?;
        }
    }
    for f in &books {
        for k in 0..f.parts() {
            let offset = k * files::PART as u64;
            let Some((total, data)) = files::read_part(store, &f.id, offset)? else { break };
            if total != f.size || data.is_empty() {
                break;
            }
            w.book_part(f, offset, &data)?;
        }
    }
    Ok(w.finish()?.0)
}

/// Добавляет копию к базе: события — как при синхронизации, вложения — если их нет.
/// Копию нужно сначала проверить целиком (`verify`): здесь записи применяются по мере чтения.
pub fn restore_into<R: Read>(mut r: Reader<R>, store: &mut Store) -> Result<Restored> {
    let mut out = Restored::default();
    while let Some(item) = r.next_item()? {
        match item {
            Item::Events(events) => out.events += store.apply_remote(&events)? as u64,
            Item::Blob { id, mime, data } => {
                if !store.has_blob(&id)? {
                    store.put_blob_checked(&id, &mime, &data)?;
                    out.blobs += 1;
                }
            }
            // Файл книги: если на устройстве его ещё нет. Испорченный не ляжет (сверка с отпечатком).
            Item::Book { file, offset, data } => {
                let fresh = store.books_dir().is_some_and(|d| files::path_in(d, &file.id).is_none());
                if fresh && files::write_part(store, &file, offset, file.size, &data).unwrap_or(false) {
                    out.books += 1;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use crate::Date;
    use std::io::Cursor;

    const FAST: Kdf = Kdf { m_kib: 8 * 1024, t: 1, p: 1 };

    fn by() -> MadeBy {
        MadeBy { app: "0.2.0".into(), created: 1_763_100_000, device: "Ноутбук".into(), device_id: "dev-a".into() }
    }

    fn jpeg(n: usize, fill: u8) -> Vec<u8> {
        let mut d = vec![fill; n];
        d[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        d
    }

    fn title(id: &str, name: &str, cover: Option<String>) -> Title {
        let mut t = Title::new(Kind::Book, name, Date::parse("2026-11-14").unwrap());
        t.id = id.into();
        t.cover = cover;
        t
    }

    /// Полка с тремя записями и двумя обложками, одна — больше куска.
    fn demo_store(dir: &Path, name: &str, device: &str) -> Store {
        let mut s = Store::open(&dir.join(name), &[5u8; 32], device).unwrap();
        let small = s.put_blob("image/jpeg", &jpeg(40_000, 7)).unwrap();
        let big = s.put_blob("image/jpeg", &jpeg((1 << 20) + 300_000, 9)).unwrap();
        s.put(KIND, "a", &title("a", "Пикник на обочине", Some(small))).unwrap();
        s.put(KIND, "b", &title("b", "Лавр", Some(big))).unwrap();
        s.put(KIND, "c", &title("c", "Брат", None)).unwrap();
        s
    }

    fn backup_of(s: &Store, key: &BackupKey) -> Vec<u8> {
        let mut out = Vec::new();
        let made = write_store(s, key, &by(), &mut out).unwrap();
        assert_eq!(made.bytes as usize, out.len());
        assert_eq!(made.blobs, 2);
        out
    }

    #[test]
    fn restores_everything_on_a_new_device() {
        let dir = tempfile::tempdir().unwrap();
        let src = demo_store(dir.path(), "a.db", "dev-a");
        let key = BackupKey::new_with("книги на даче", FAST).unwrap();
        let file = backup_of(&src, &key);
        assert_eq!(&file[..8], MAGIC);

        let checked = verify(Reader::open(Cursor::new(&file), "книги на даче").unwrap(), "dev-b", |_| Ok(false)).unwrap();
        assert_eq!((checked.events, checked.blobs, checked.conflicts), (src.event_count().unwrap(), 2, 0));
        assert_eq!(checked.manifest.counts.titles, 3);
        assert_eq!((checked.manifest.device.as_str(), checked.manifest.app.as_str()), ("Ноутбук", "0.2.0"));

        let mut dst = Store::open(&dir.path().join("b.db"), &[6u8; 32], "dev-b").unwrap();
        let r = restore_into(Reader::open(Cursor::new(&file), "книги на даче").unwrap(), &mut dst).unwrap();
        assert_eq!(r, Restored { events: src.event_count().unwrap(), blobs: 2, books: 0 });
        assert_eq!(dst.list::<Title>(KIND).unwrap(), src.list::<Title>(KIND).unwrap());
        assert!(dst.missing_blobs().unwrap().is_empty(), "обложки на месте");
        // Новые записи на новом устройстве идут под его номером и не спорят со старыми.
        dst.put(KIND, "d", &title("d", "Щегол", None)).unwrap();
        assert_eq!(dst.versions().unwrap().get("dev-b"), Some(&1));
    }

    #[test]
    fn merge_adds_only_missing_and_keeps_newer_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = demo_store(dir.path(), "a.db", "dev-a");
        let key = BackupKey::new_with("книги на даче", FAST).unwrap();
        let file = backup_of(&src, &key);
        // После копии запись поправили, а другую удалили — восстановление старой копии это не откатит.
        let mut b = src.get::<Title>(KIND, "b").unwrap().unwrap();
        b.rating = Some(10);
        src.put(KIND, "b", &b).unwrap();
        src.delete(KIND, "c").unwrap();
        let before = src.event_count().unwrap();
        let r = restore_into(Reader::open(Cursor::new(&file), "книги на даче").unwrap(), &mut src).unwrap();
        assert_eq!(r, Restored::default(), "всё из копии уже есть");
        assert_eq!(src.event_count().unwrap(), before);
        assert_eq!(src.get::<Title>(KIND, "b").unwrap().unwrap().rating, Some(10));
        assert!(src.get::<Title>(KIND, "c").unwrap().is_none());
    }

    /// Файлы книг идут в копию только по своему переключателю и восстанавливаются целиком.
    #[test]
    fn book_files_in_backup_only_when_chosen() {
        use crate::files::{BookFile, BACKUP_KEY, FILE, PART};
        use crate::reader::BookFormat;
        let dir = tempfile::tempdir().unwrap();
        let mut s = demo_store(dir.path(), "a.db", "aaaa");
        s.set_books_dir(dir.path().join("a-books"));
        let data: Vec<u8> = (0..(PART + 500)).map(|i| (i % 239) as u8).collect();
        let id = crate::db::blob_id(&data);
        let d = s.books_dir().unwrap().to_path_buf();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{id}.epub")), &data).unwrap();
        let t = s.list::<Title>(KIND).unwrap()[0].id.clone();
        s.put(FILE, &id, &BookFile { id: id.clone(), title: t, name: "b.epub".into(), format: BookFormat::Epub, size: data.len() as u64, added: 1 }).unwrap();
        let key = BackupKey::new_with("книги на даче", FAST).unwrap();
        let mut small = Vec::new();
        assert_eq!(write_store(&s, &key, &by(), &mut small).unwrap().books, 0, "без переключателя файлов книг в копии нет");
        s.set_meta(BACKUP_KEY, "1").unwrap();
        let mut big = Vec::new();
        let made = write_store(&s, &key, &by(), &mut big).unwrap();
        assert_eq!(made.books, 1);
        assert!(big.len() >= small.len() + data.len());
        let checked = verify(Reader::open(Cursor::new(&big), "книги на даче").unwrap(), "", |_| Ok(false)).unwrap();
        assert_eq!((checked.books, checked.manifest.counts.books), (1, 1));
        // Новое устройство: записи и файл книги.
        let mut fresh = Store::open(&dir.path().join("b.db"), &[3u8; 32], "bbbb").unwrap();
        fresh.set_books_dir(dir.path().join("b-books"));
        let r = restore_into(Reader::open(Cursor::new(&big), "книги на даче").unwrap(), &mut fresh).unwrap();
        assert_eq!(r.books, 1);
        let got = crate::files::path_in(fresh.books_dir().unwrap(), &id).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), data);
        // Повторное восстановление ничего не добавляет.
        let r = restore_into(Reader::open(Cursor::new(&big), "книги на даче").unwrap(), &mut fresh).unwrap();
        assert_eq!(r.books, 0);
    }

    #[test]
    fn same_device_restarted_under_old_number_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let src = demo_store(dir.path(), "a.db", "dev-a");
        let key = BackupKey::new_with("книги на даче", FAST).unwrap();
        let file = backup_of(&src, &key);
        // База начата заново под тем же номером устройства и уже с новой записью.
        let mut fresh = Store::open(&dir.path().join("c.db"), &[7u8; 32], "dev-a").unwrap();
        fresh.put(KIND, "z", &title("z", "Другая книга", None)).unwrap();
        let checked = verify(Reader::open(Cursor::new(&file), "книги на даче").unwrap(), "dev-a", |ev| conflicts_with(&fresh, ev)).unwrap();
        assert_eq!(checked.conflicts, 1);
        // Та же база, что сделала копию, расхождений не находит.
        let checked = verify(Reader::open(Cursor::new(&file), "книги на даче").unwrap(), "dev-a", |ev| conflicts_with(&src, ev)).unwrap();
        assert_eq!(checked.conflicts, 0);
    }
}

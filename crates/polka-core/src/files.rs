//! Файлы книг на всех устройствах.
//!
//! О каждом файле, открытом в читалке, в журнале есть запись `file` (номер — SHA-256 файла): к какой
//! записи полки он относится, как назывался, какого формата и размера. Записи расходятся по устройствам
//! вместе с полкой; сами файлы — только там, где включён обмен файлами (`files.sync`, у каждого устройства
//! свой): по Wi-Fi и через облако, кусками по 8 МБ. Недокачанный файл остаётся рядом как `.part`
//! и в следующий раз докачивается с того же места; готовый проверяется по SHA-256 — номер файла и есть
//! его отпечаток, подменить содержимое нельзя. В резервную копию файлы идут по своему переключателю
//! (`files.backup`).
//!
//! Файл лежит в папке книг устройства (`<sha256>.<ext>`), о том, что он здесь есть, говорит местная
//! запись `book.<sha256>`. Убранный с устройства файл помечается (`bookskip.<sha256>`) и обратно сам
//! не приходит.

use crate::db::Store;
use crate::error::{Error, Result};
use crate::model::{Title, KIND};
use crate::reader::{BookFormat, MAX_BOOK};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Записи журнала о файлах книг.
pub const FILE: &str = "file";
/// Кусок файла при передаче.
pub const PART: usize = 8 << 20;
/// Местные настройки: обмен файлами и файлы в резервной копии.
pub const SYNC_KEY: &str = "files.sync";
pub const BACKUP_KEY: &str = "files.backup";
/// Местные записи: файл есть на этом устройстве / убран с него.
pub const LOCAL: &str = "book.";
const SKIP: &str = "bookskip.";
/// Расширения файлов книг в папке.
pub const EXTS: [&str; 4] = ["epub", "fb2", "fb2.zip", "pdf"];

/// Файл книги — запись журнала `file`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookFile {
    /// SHA-256 файла.
    pub id: String,
    /// Запись полки.
    pub title: String,
    /// Имя файла, как его выбрали.
    pub name: String,
    pub format: BookFormat,
    pub size: u64,
    /// Когда файл открыли впервые (секунды Unix).
    pub added: i64,
}

impl BookFile {
    pub fn check(&self) -> Result<()> {
        if !is_hash(&self.id) || self.title.is_empty() || self.title.len() > 64 || self.name.chars().count() > 200 || self.size == 0 || self.size > MAX_BOOK as u64 {
            return Err(Error::Format("неверная запись о файле книги".into()));
        }
        Ok(())
    }

    /// Сколько кусков в файле.
    pub fn parts(&self) -> u64 {
        self.size.div_ceil(PART as u64).max(1)
    }
}

/// Файл на этом устройстве — местная запись `book.<sha256>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Local {
    pub title: String,
    pub name: String,
    pub format: BookFormat,
    pub size: u64,
    pub added: i64,
}

pub fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Файл книги в папке, если он там есть.
pub fn path_in(dir: &Path, hash: &str) -> Option<PathBuf> {
    if !is_hash(hash) {
        return None;
    }
    EXTS.iter().map(|e| dir.join(format!("{hash}.{e}"))).find(|p| p.is_file())
}

/// Расширение файла: по формату и началу файла (FB2 бывает в архиве).
pub fn ext_for(format: BookFormat, head: &[u8]) -> &'static str {
    match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Fb2 if head.starts_with(b"PK\x03\x04") => "fb2.zip",
        BookFormat::Fb2 => "fb2",
    }
}

fn dir_of(st: &Store) -> Result<&Path> {
    st.books_dir().ok_or_else(|| Error::Format("папка книг не задана".into()))
}

pub fn local(st: &Store, hash: &str) -> Option<Local> {
    st.meta(&format!("{LOCAL}{hash}")).ok().flatten().and_then(|v| serde_json::from_str(&v).ok())
}

pub fn set_local(st: &Store, hash: &str, l: &Local) -> Result<()> {
    st.set_meta(&format!("{LOCAL}{hash}"), &serde_json::to_string(l)?)
}

/// Файл убрали с этого устройства — обратно сам не приходит.
pub fn skipped(st: &Store, hash: &str) -> bool {
    st.meta(&format!("{SKIP}{hash}")).ok().flatten().is_some()
}

pub fn set_skipped(st: &Store, hash: &str, skip: bool) -> Result<()> {
    if skip {
        st.set_meta(&format!("{SKIP}{hash}"), "1")
    } else {
        st.delete_meta(&format!("{SKIP}{hash}"))
    }
}

fn flag(st: &Store, key: &str) -> bool {
    st.meta(key).ok().flatten().as_deref() == Some("1")
}

/// Обмен файлами книг включён на этом устройстве.
pub fn sync_on(st: &Store) -> bool {
    st.books_dir().is_some() && flag(st, SYNC_KEY)
}

/// Файлы книг идут в резервную копию.
pub fn backup_on(st: &Store) -> bool {
    st.books_dir().is_some() && flag(st, BACKUP_KEY)
}

fn alive(st: &Store, title: &str) -> bool {
    st.get::<Title>(KIND, title).ok().flatten().is_some()
}

/// Файлы, известные журналу, которые есть на этом устройстве (их можно отдать).
pub fn present(st: &Store) -> Result<Vec<BookFile>> {
    let dir = dir_of(st)?;
    let mut list = st.list::<BookFile>(FILE)?;
    list.retain(|f| f.check().is_ok() && path_in(dir, &f.id).is_some());
    list.sort_by_key(|f| f.added);
    Ok(list)
}

/// Файлы, которых здесь нет, но которые нужны: запись полки жива, файл не убран отсюда.
pub fn wanted(st: &Store) -> Result<Vec<BookFile>> {
    let dir = dir_of(st)?;
    let mut list = st.list::<BookFile>(FILE)?;
    list.retain(|f| f.check().is_ok() && path_in(dir, &f.id).is_none() && !skipped(st, &f.id) && alive(st, &f.title));
    list.sort_by_key(|f| f.added);
    Ok(list)
}

/// Файлы, которых здесь нет (для карточки: «файл на другом устройстве»).
pub fn elsewhere(st: &Store) -> Result<Vec<BookFile>> {
    let dir = dir_of(st)?;
    let mut list = st.list::<BookFile>(FILE)?;
    list.retain(|f| f.check().is_ok() && path_in(dir, &f.id).is_none() && alive(st, &f.title));
    Ok(list)
}

/// Кусок файла для отдачи: размер файла и до `PART` байт с места `offset`. `None` — файла нет.
pub fn read_part(st: &Store, hash: &str, offset: u64) -> Result<Option<(u64, Vec<u8>)>> {
    let Some(path) = path_in(dir_of(st)?, hash) else { return Ok(None) };
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    if offset > total {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; (total - offset).min(PART as u64) as usize];
    f.read_exact(&mut buf)?;
    Ok(Some((total, buf)))
}

fn part_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{hash}.part"))
}

/// Сколько файла уже получено (`.part`): с этого места его и докачивать.
pub fn partial(st: &Store, hash: &str) -> u64 {
    let Ok(dir) = dir_of(st) else { return 0 };
    std::fs::metadata(part_path(dir, hash)).map(|m| m.len()).unwrap_or(0)
}

/// Принять кусок файла. Куски идут по порядку: `offset` — ровно сколько уже получено (с нуля — начать
/// заново). На последнем куске файл сверяется с отпечатком и встаёт на место; тогда — `true`.
pub fn write_part(st: &Store, f: &BookFile, offset: u64, total: u64, data: &[u8]) -> Result<bool> {
    f.check()?;
    let dir = dir_of(st)?;
    if path_in(dir, &f.id).is_some() {
        return Ok(true);
    }
    if total != f.size || data.is_empty() || data.len() > PART || offset + data.len() as u64 > total {
        return Err(Error::Format("кусок файла книги не подходит".into()));
    }
    std::fs::create_dir_all(dir)?;
    let part = part_path(dir, &f.id);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if offset != have && offset != 0 {
        return Err(Error::Format("кусок файла книги не по порядку".into()));
    }
    let mut out = OpenOptions::new().create(true).write(true).truncate(offset == 0).open(&part)?;
    out.seek(SeekFrom::Start(offset))?;
    out.write_all(data)?;
    out.flush()?;
    drop(out);
    if offset + (data.len() as u64) < total {
        return Ok(false);
    }
    // Последний кусок: сверить отпечаток и положить на место.
    let mut file = File::open(&part)?;
    let mut hasher = Sha256::new();
    let mut head = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if head.len() < 8 {
            head.extend_from_slice(&buf[..n.min(8)]);
        }
        hasher.update(&buf[..n]);
    }
    drop(file);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != f.id {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Format("файл книги пришёл испорченным".into()));
    }
    std::fs::rename(&part, dir.join(format!("{}.{}", f.id, ext_for(f.format, &head))))?;
    set_local(st, &f.id, &Local { title: f.title.clone(), name: f.name.clone(), format: f.format, size: f.size, added: crate::now_secs() })?;
    Ok(true)
}

/// Файлы, открытые до 0.8.0 (журнал о них не знал): завести о них записи. Возвращает, сколько завели.
pub fn adopt(st: &mut Store) -> Result<usize> {
    let Some(dir) = st.books_dir().map(Path::to_path_buf) else { return Ok(0) };
    let mut n = 0;
    for (key, value) in st.meta_prefix(LOCAL)? {
        let hash = &key[LOCAL.len()..];
        let Ok(l) = serde_json::from_str::<Local>(&value) else { continue };
        if path_in(&dir, hash).is_none() || st.get::<BookFile>(FILE, hash)?.is_some() || !alive(st, &l.title) {
            continue;
        }
        let f = BookFile { id: hash.to_string(), title: l.title, name: l.name, format: l.format, size: l.size, added: l.added };
        if f.check().is_ok() {
            st.put(FILE, hash, &f)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::blob_id;
    use crate::model::Kind;
    use crate::Date;

    fn store(dir: &Path, name: &str) -> Store {
        let mut s = Store::open(&dir.join(format!("{name}.db")), &[7u8; 32], name).unwrap();
        s.set_books_dir(dir.join(format!("{name}-books")));
        let mut t = Title::new(Kind::Book, "Пикник на обочине", Date::parse("2026-11-01").unwrap());
        t.id = "t1".into();
        s.put(KIND, "t1", &t).unwrap();
        s
    }

    #[test]
    fn file_goes_in_parts_and_is_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let a = store(tmp.path(), "a");
        let b = store(tmp.path(), "b");
        let data: Vec<u8> = (0..(PART * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        let id = blob_id(&data);
        let dir_a = a.books_dir().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::write(dir_a.join(format!("{id}.pdf")), &data).unwrap();
        let f = BookFile { id: id.clone(), title: "t1".into(), name: "picnic.pdf".into(), format: BookFormat::Pdf, size: data.len() as u64, added: 1 };
        assert_eq!(f.parts(), 3);
        // Первый кусок, обрыв, докачка с того же места.
        let (total, p0) = read_part(&a, &id, 0).unwrap().unwrap();
        assert_eq!(total, data.len() as u64);
        assert!(!write_part(&b, &f, 0, total, &p0).unwrap());
        assert_eq!(partial(&b, &id), PART as u64);
        let (_, p1) = read_part(&a, &id, PART as u64).unwrap().unwrap();
        assert!(write_part(&b, &f, PART as u64 * 2, total, &p1).is_err(), "не по порядку");
        assert!(!write_part(&b, &f, PART as u64, total, &p1).unwrap());
        let (_, p2) = read_part(&a, &id, PART as u64 * 2).unwrap().unwrap();
        assert_eq!(p2.len(), 1234);
        assert!(write_part(&b, &f, PART as u64 * 2, total, &p2).unwrap());
        let got = path_in(b.books_dir().unwrap(), &id).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), data);
        assert!(got.to_string_lossy().ends_with(".pdf"));
        assert_eq!(local(&b, &id).unwrap().title, "t1");
        assert_eq!(partial(&b, &id), 0);
    }

    #[test]
    fn spoiled_file_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let b = store(tmp.path(), "b");
        let data = b"%PDF-1.4 tiny".to_vec();
        let f = BookFile { id: blob_id("другое содержимое".as_bytes()), title: "t1".into(), name: "x.pdf".into(), format: BookFormat::Pdf, size: data.len() as u64, added: 1 };
        assert!(write_part(&b, &f, 0, f.size, &data).is_err());
        assert!(path_in(b.books_dir().unwrap(), &f.id).is_none());
        assert_eq!(partial(&b, &f.id), 0, "испорченный кусок удалён");
    }

    #[test]
    fn wanted_present_skipped_and_adopted() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = store(tmp.path(), "a");
        let dir = a.books_dir().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let data = b"PK\x03\x04 fb2 in zip".to_vec();
        let id = blob_id(&data);
        std::fs::write(dir.join(format!("{id}.fb2.zip")), &data).unwrap();
        // Файл открыли в 0.7.0: местная запись есть, записи журнала нет.
        set_local(&a, &id, &Local { title: "t1".into(), name: "p.fb2.zip".into(), format: BookFormat::Fb2, size: data.len() as u64, added: 5 }).unwrap();
        assert_eq!(adopt(&mut a).unwrap(), 1);
        assert_eq!(adopt(&mut a).unwrap(), 0, "второй раз не заводится");
        assert_eq!(present(&a).unwrap().len(), 1);
        assert!(wanted(&a).unwrap().is_empty());
        // Другое устройство: запись пришла с журналом, файла нет.
        let mut b = store(tmp.path(), "b");
        let rec = a.get::<BookFile>(FILE, &id).unwrap().unwrap();
        b.put(FILE, &id, &rec).unwrap();
        assert_eq!(wanted(&b).unwrap().len(), 1);
        set_skipped(&b, &id, true).unwrap();
        assert!(wanted(&b).unwrap().is_empty(), "убранный не приходит");
        assert_eq!(elsewhere(&b).unwrap().len(), 1);
        set_skipped(&b, &id, false).unwrap();
        b.delete(KIND, "t1").unwrap();
        assert!(wanted(&b).unwrap().is_empty(), "запись полки удалена — файл не нужен");
        assert!(!sync_on(&b));
        b.set_meta(SYNC_KEY, "1").unwrap();
        assert!(sync_on(&b));
        assert_eq!(ext_for(BookFormat::Fb2, &data), "fb2.zip");
    }
}

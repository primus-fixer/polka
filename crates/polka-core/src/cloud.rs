//! Облако: Яндекс Диск или свой сервер WebDAV — общий ящик для устройств одного человека.
//! Устроено как в «Кармане»: тот же формат, свои имена и метки.
//!
//! Облако не знает, что лежит в файлах: всё шифруется на устройстве до отправки.
//! - `polka.key` — ключ облака (случайный, 256 бит), зашифрованный Argon2id от пароля копий.
//!   По паролю новое устройство получает ключ облака; сам пароль нигде не хранится.
//! - `e/<устройство>-<с>-<по>.pev` — события одного устройства с номерами «с…по», сжатые
//!   и зашифрованные XChaCha20-Poly1305. Имя файла входит в проверку подлинности:
//!   переименованный или подменённый файл не расшифруется.
//! - `b/<имя>.pbl` — обложка. Имя — MAC от отпечатка снимка: по имени нельзя проверить,
//!   лежит ли в облаке известная обложка.
//! - `k/<имя>-<кусок>-<всего>.pbk` — файл книги кусками по 8 МБ (с 0.8.0, если на устройстве включён
//!   обмен файлами книг). Имя — MAC от отпечатка файла; номер куска и их число входят в проверку
//!   подлинности. Файл забирается, когда в облаке лежат все его куски, и сверяется с отпечатком.
//!
//! Общего состояния у устройств нет: по именам файлов видно, какие события есть в облаке.
//! Устройство забирает файлы, которые продолжают его журнал без пропусков, и отправляет то,
//! чего в облаке нет, — в том числе события других устройств, полученные по Wi-Fi.
//! Мелкие файлы одного устройства время от времени сливаются в крупные.

use crate::backup::{self, Kdf};
use crate::db::{self, Event, Store};
use crate::files::{self, BookFile};
use crate::error::{CloudFail, Error, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use blake2::digest::consts::U32;
use blake2::digest::Mac;
use blake2::Blake2bMac;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Файл ключа облака в папке «Полки».
pub const KEY_FILE: &str = "polka.key";
const KEY_FORMAT: u16 = 1;
const FILE_MAGIC: &[u8; 4] = b"PLKC";
const FILE_VERSION: u8 = 1;
const KIND_EVENTS: u8 = b'e';
const KIND_BLOB: u8 = b'b';
const KIND_BOOK: u8 = b'k';
const EVENTS_EXT: &str = ".pev";
const BLOB_EXT: &str = ".pbl";
const BOOK_EXT: &str = ".pbk";
/// Магия, версия, вид, nonce и метка подлинности.
const OVERHEAD: usize = 4 + 1 + 1 + 24 + 16;
/// Событий в одном файле.
pub const FILE_EVENTS: u64 = 5000;
/// Больше стольких файлов одного устройства — пора сливать.
pub const COMPACT_AFTER: usize = 12;
/// Предел файла событий в облаке и после распаковки.
pub const MAX_EVENTS_FILE: usize = 64 << 20;
const MAX_PLAIN: usize = 256 << 20;
/// Предел файла обложки: снимок и немного на заголовок.
pub const MAX_BLOB_FILE: usize = db::MAX_BLOB + 1024;
pub const MAX_KEY_FILE: usize = 64 << 10;
/// Предел куска файла книги в облаке.
pub const MAX_BOOK_FILE: usize = files::PART + OVERHEAD + 64;
/// Полный список обложек в облаке сверяем не чаще раза в сутки, если нет новых.
const BLOBS_RELIST: i64 = 24 * 3600;

fn fail(kind: CloudFail, msg: impl Into<String>) -> Error {
    Error::Cloud(kind, msg.into())
}

fn gone() -> Error {
    fail(CloudFail::Gone, "Данные «Полки» в облаке удалены. Чтобы продолжить обмен, подключите облако заново")
}

fn replaced() -> Error {
    fail(CloudFail::Replaced, "Облако подключили заново на другом устройстве. Введите пароль копий, чтобы продолжить обмен")
}

fn conflict() -> Error {
    fail(
        CloudFail::Conflict,
        "В облаке записи этого устройства расходятся с базой: его база начиналась заново под тем же номером. Обмен остановлен, чтобы ничего не потерять",
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn valid_group(g: &str) -> bool {
    g.len() == 32 && g.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Папка внутри папки «Полки» в облаке.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dir {
    Root,
    Events,
    Blobs,
    /// Файлы книг (с 0.8.0).
    Books,
}

impl Dir {
    /// Имя папки: пусто — сама папка «Полки».
    pub fn folder(self) -> &'static str {
        match self {
            Dir::Root => "",
            Dir::Events => "e",
            Dir::Blobs => "b",
            Dir::Books => "k",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
}

/// Облачное хранилище: папка «Полки» с подпапками для событий и обложек.
pub trait Cloud {
    /// Файлы в папке; если папки нет — пустой список.
    fn list(&self, dir: Dir) -> Result<Vec<Entry>>;
    /// Содержимое файла не больше `max` байт; `None` — файла нет.
    fn get(&self, dir: Dir, name: &str, max: usize) -> Result<Option<Vec<u8>>>;
    /// Записывает файл целиком: новый или поверх прежнего.
    fn put(&self, dir: Dir, name: &str, data: &[u8]) -> Result<()>;
    /// Файл есть.
    fn exists(&self, dir: Dir, name: &str) -> Result<bool>;
    /// Удаляет файл; если его уже нет — не ошибка.
    fn delete(&self, dir: Dir, name: &str) -> Result<()>;
}

// ——— Ключ облака ———

/// Файл ключа в облаке: параметры Argon2id, соль и зашифрованный ключ облака.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyFile {
    pub format: u16,
    /// Номер облака: меняется, только если облако подключают заново с нуля.
    pub group: String,
    pub m: u32,
    pub t: u32,
    pub p: u32,
    pub salt: String,
    pub nonce: String,
    pub key: String,
    /// Когда задан пароль: секунды Unix.
    #[serde(default)]
    pub changed: i64,
}

impl KeyFile {
    pub fn parse(data: &[u8]) -> Result<KeyFile> {
        let kf: KeyFile = serde_json::from_slice(data).map_err(|_| fail(CloudFail::Format, "Ключ облака повреждён"))?;
        if kf.format > KEY_FORMAT {
            return Err(fail(CloudFail::Newer, "Облако подключено более новой версией «Полки» — обновите приложение"));
        }
        if kf.format == 0 || !valid_group(&kf.group) {
            return Err(fail(CloudFail::Format, "Ключ облака повреждён"));
        }
        Ok(kf)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    fn kdf(&self) -> Kdf {
        Kdf { m_kib: self.m, t: self.t, p: self.p }
    }

    /// Параметры и номер облака защищены вместе с ключом: подменить их незаметно нельзя.
    fn aad(&self) -> Vec<u8> {
        format!("polka-cloud-key/{}|{}|{}|{}|{}|{}", self.format, self.group, self.m, self.t, self.p, self.salt).into_bytes()
    }
}

/// Ключ облака. Хранится в базе устройства (она сама зашифрована) — для обмена пароль не нужен.
pub struct CloudKey {
    group: String,
    key: [u8; 32],
}

impl Drop for CloudKey {
    fn drop(&mut self) {
        backup::wipe(&mut self.key);
    }
}

fn mac256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Blake2bMac::<U32>::new_with_salt_and_personal(key, &[], b"polka-cloud").expect("ключ MAC — 32 байта");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

impl CloudKey {
    /// Новый ключ для облака, которое подключают впервые.
    pub fn generate() -> Result<CloudKey> {
        let group: [u8; 16] = backup::random()?;
        Ok(CloudKey { group: hex(&group), key: backup::random()? })
    }

    pub fn group(&self) -> &str {
        &self.group
    }

    /// Для хранения в базе: «1:номер облака:ключ».
    pub fn to_meta(&self) -> String {
        format!("1:{}:{}", self.group, B64.encode(self.key))
    }

    pub fn from_meta(s: &str) -> Result<CloudKey> {
        let bad = || fail(CloudFail::Format, "Ключ облака на устройстве повреждён: подключите облако заново");
        let p: Vec<&str> = s.split(':').collect();
        if p.len() != 3 || p[0] != "1" || !valid_group(p[1]) {
            return Err(bad());
        }
        let mut raw = B64.decode(p[2]).map_err(|_| bad())?;
        let key = <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| bad());
        backup::wipe(&mut raw);
        Ok(CloudKey { group: p[1].to_string(), key: key? })
    }

    /// Файл ключа для облака: ключ облака, зашифрованный паролем копий (около секунды).
    pub fn key_file(&self, password: &str, kdf: Kdf, changed: i64) -> Result<KeyFile> {
        backup::check_password(password)?;
        if !kdf.valid() {
            return Err(fail(CloudFail::Format, "неподходящие параметры ключа"));
        }
        let salt: [u8; 16] = backup::random()?;
        let nonce: [u8; 24] = backup::random()?;
        let mut kf = KeyFile {
            format: KEY_FORMAT,
            group: self.group.clone(),
            m: kdf.m_kib,
            t: kdf.t,
            p: kdf.p,
            salt: B64.encode(salt),
            nonce: B64.encode(nonce),
            key: String::new(),
            changed,
        };
        let mut k = backup::derive_raw(password, &salt, kdf)?;
        let wrapped = XChaCha20Poly1305::new((&k).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: &self.key, aad: &kf.aad() });
        backup::wipe(&mut k);
        kf.key = B64.encode(wrapped.map_err(|_| Error::Decrypt)?);
        Ok(kf)
    }

    /// Ключ облака из файла ключа по паролю копий (около секунды).
    pub fn from_key_file(kf: &KeyFile, password: &str) -> Result<CloudKey> {
        let bad = || fail(CloudFail::Format, "Ключ облака повреждён");
        if !kf.kdf().valid() {
            return Err(fail(CloudFail::Format, "Ключ облака не открывается: неподходящие параметры"));
        }
        let salt = <[u8; 16]>::try_from(B64.decode(&kf.salt).map_err(|_| bad())?.as_slice()).map_err(|_| bad())?;
        let nonce = <[u8; 24]>::try_from(B64.decode(&kf.nonce).map_err(|_| bad())?.as_slice()).map_err(|_| bad())?;
        let wrapped = B64.decode(&kf.key).map_err(|_| bad())?;
        if wrapped.len() != 32 + 16 {
            return Err(bad());
        }
        let mut k = backup::derive_raw(password, &salt, kf.kdf())?;
        let opened = XChaCha20Poly1305::new((&k).into()).decrypt(XNonce::from_slice(&nonce), Payload { msg: &wrapped, aad: &kf.aad() });
        backup::wipe(&mut k);
        let mut raw = opened.map_err(|_| Error::CloudPassword)?;
        let key = <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| Error::CloudPassword);
        backup::wipe(&mut raw);
        Ok(CloudKey { group: kf.group.clone(), key: key? })
    }

    /// Свой ключ для каждой задачи: шифрование файлов и имена обложек.
    fn sub(&self, label: &str) -> [u8; 32] {
        mac256(&self.key, label.as_bytes())
    }

    /// Имя файла обложки в облаке: по нему не узнать отпечаток снимка.
    pub fn blob_name(&self, id: &str) -> String {
        let mut k = self.sub("names");
        let name = mac256(&k, id.as_bytes());
        backup::wipe(&mut k);
        format!("{}{BLOB_EXT}", &hex(&name)[..32])
    }

    /// Имя куска файла книги в облаке: по нему не узнать отпечаток файла.
    pub fn book_name(&self, id: &str, part: u64, parts: u64) -> String {
        let mut k = self.sub("books");
        let name = mac256(&k, id.as_bytes());
        backup::wipe(&mut k);
        format!("{}-{part}-{parts}{BOOK_EXT}", &hex(&name)[..32])
    }
}

fn hex32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Кусок файла книги: имя, номер куска, всего кусков.
fn parse_book_name(name: &str) -> Option<(&str, u64, u64)> {
    let mut it = name.strip_suffix(BOOK_EXT)?.split('-');
    let (mac, part, parts) = (it.next()?, it.next()?.parse::<u64>().ok()?, it.next()?.parse::<u64>().ok()?);
    (it.next().is_none() && hex32(mac) && part < parts && parts <= 64).then_some((mac, part, parts))
}

/// Файл ключа в облаке; `None` — его нет.
pub fn read_key_file(cloud: &dyn Cloud) -> Result<Option<KeyFile>> {
    cloud.get(Dir::Root, KEY_FILE, MAX_KEY_FILE)?.map(|b| KeyFile::parse(&b)).transpose()
}

/// Файл «Полки» в папке облака. Удаляются только такие: чужие файлы в «Полка/e» и «Полка/b»
/// (если человек положил их туда сам) остаются.
fn ours(dir: Dir, name: &str) -> bool {
    match dir {
        Dir::Root => name == KEY_FILE,
        Dir::Events => parse_event_name(name).is_some(),
        Dir::Blobs => name.strip_suffix(BLOB_EXT).is_some_and(hex32),
        Dir::Books => parse_book_name(name).is_some(),
    }
}

/// Подключает устройство по паролю копий: ключ облака уже есть — открывает его, нет — создаёт.
/// Возвращает ключ и `true`, если облако подключено впервые.
pub fn join(cloud: &dyn Cloud, password: &str, kdf: Kdf, now: i64) -> Result<(CloudKey, bool)> {
    backup::check_password(password)?;
    if let Some(kf) = read_key_file(cloud)? {
        return Ok((CloudKey::from_key_file(&kf, password)?, false));
    }
    // Без ключа старые файлы «Полки» не прочитать никому: убираем их, чтобы не мешали.
    for dir in [Dir::Events, Dir::Blobs, Dir::Books] {
        for e in cloud.list(dir)?.into_iter().filter(|e| ours(dir, &e.name)) {
            cloud.delete(dir, &e.name)?;
        }
    }
    let key = CloudKey::generate()?;
    let kf = key.key_file(password, kdf, now)?;
    cloud.put(Dir::Root, KEY_FILE, &kf.to_bytes())?;
    // Если в ту же минуту облако подключило другое устройство, действует записанный последним ключ.
    match read_key_file(cloud)? {
        Some(cur) if cur.group == key.group => Ok((key, true)),
        Some(cur) => Ok((CloudKey::from_key_file(&cur, password)?, false)),
        None => Err(gone()),
    }
}

/// Новый пароль копий: тот же ключ облака, зашифрованный новым паролем.
pub fn put_key_file(cloud: &dyn Cloud, key: &CloudKey, kf: &KeyFile) -> Result<()> {
    if kf.group != key.group {
        return Err(replaced());
    }
    match read_key_file(cloud)? {
        None => Err(gone()),
        Some(cur) if cur.group != key.group => Err(replaced()),
        Some(_) => cloud.put(Dir::Root, KEY_FILE, &kf.to_bytes()),
    }
}

/// Удаляет данные «Полки» из облака. Сначала ключ: другие устройства увидят это и перестанут слать файлы.
pub fn wipe(cloud: &dyn Cloud, cancel: &AtomicBool) -> Result<usize> {
    cloud.delete(Dir::Root, KEY_FILE)?;
    let mut n = 1;
    for dir in [Dir::Events, Dir::Blobs, Dir::Books] {
        for e in cloud.list(dir)?.into_iter().filter(|e| ours(dir, &e.name)) {
            if cancel.load(Ordering::Relaxed) {
                return Err(fail(CloudFail::Cancelled, "Удаление остановлено"));
            }
            cloud.delete(dir, &e.name)?;
            n += 1;
        }
    }
    Ok(n)
}

// ——— Файлы ———

fn aad(kind: u8, group: &str, name: &str) -> Vec<u8> {
    format!("polka-cloud/1|{}|{group}|{name}", kind as char).into_bytes()
}

fn seal_file(key: &CloudKey, kind: u8, name: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let nonce: [u8; 24] = backup::random()?;
    let mut k = key.sub("files");
    let ct = XChaCha20Poly1305::new((&k).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: &aad(kind, &key.group, name) });
    backup::wipe(&mut k);
    let ct = ct.map_err(|_| Error::Decrypt)?;
    let mut out = Vec::with_capacity(OVERHEAD + plain.len());
    out.extend_from_slice(FILE_MAGIC);
    out.push(FILE_VERSION);
    out.push(kind);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

fn open_file(key: &CloudKey, kind: u8, name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let cover = kind == KIND_BLOB;
    if data.len() < OVERHEAD || &data[..4] != FILE_MAGIC || data[5] != kind {
        let what = if cover { "обложка в облаке не от «Полки» или повреждена" } else { "файл в облаке не от «Полки» или повреждён" };
        return Err(fail(CloudFail::Format, format!("{what} ({name})")));
    }
    if data[4] > FILE_VERSION {
        return Err(fail(CloudFail::Newer, "В облаке файлы более новой версии «Полки» — обновите приложение"));
    }
    let mut k = key.sub("files");
    let plain = XChaCha20Poly1305::new((&k).into()).decrypt(XNonce::from_slice(&data[6..30]), Payload { msg: &data[30..], aad: &aad(kind, &key.group, name) });
    backup::wipe(&mut k);
    let what = if cover { "обложка в облаке повреждена или подменена" } else { "файл в облаке повреждён или подменён" };
    plain.map_err(|_| fail(CloudFail::Format, format!("{what} ({name})")))
}

/// Имя файла событий: устройство и номера событий «с…по».
pub fn event_name(author: &str, from: u64, to: u64) -> String {
    format!("{author}-{from}-{to}{EVENTS_EXT}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFileName {
    pub author: String,
    pub from: u64,
    pub to: u64,
}

/// Разбирает имя файла событий. Чужие и неканонические имена («007» вместо «7») не подходят.
pub fn parse_event_name(name: &str) -> Option<EventFileName> {
    let stem = name.strip_suffix(EVENTS_EXT)?;
    let mut parts = stem.split('-');
    let author = parts.next()?;
    let from: u64 = parts.next()?.parse().ok()?;
    let to: u64 = parts.next()?.parse().ok()?;
    let ok_author = !author.is_empty() && author.len() <= 64 && author.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if parts.next().is_some() || !ok_author || from == 0 || to < from || to > i64::MAX as u64 || event_name(author, from, to) != name {
        return None;
    }
    Some(EventFileName { author: author.to_string(), from, to })
}

#[derive(Serialize)]
struct EventFileOut<'a> {
    author: &'a str,
    from: u64,
    to: u64,
    events: &'a [Event],
}

#[derive(Deserialize)]
struct EventFileIn {
    author: String,
    from: u64,
    to: u64,
    events: Vec<Event>,
}

fn seal_events(key: &CloudKey, author: &str, from: u64, to: u64, events: &[Event]) -> Result<(String, Vec<u8>)> {
    let name = event_name(author, from, to);
    let json = serde_json::to_vec(&EventFileOut { author, from, to, events })?;
    let packed = miniz_oxide::deflate::compress_to_vec(&json, 6);
    let data = seal_file(key, KIND_EVENTS, &name, &packed)?;
    Ok((name, data))
}

/// Расшифровывает файл событий и проверяет, что внутри ровно то, что обещает имя.
fn open_events(key: &CloudKey, author: &str, from: u64, to: u64, data: &[u8]) -> Result<Vec<Event>> {
    let name = event_name(author, from, to);
    let bad = || fail(CloudFail::Format, format!("файл в облаке повреждён ({name})"));
    let packed = open_file(key, KIND_EVENTS, &name, data)?;
    let json = miniz_oxide::inflate::decompress_to_vec_with_limit(&packed, MAX_PLAIN).map_err(|_| bad())?;
    let f: EventFileIn = serde_json::from_slice(&json).map_err(|_| bad())?;
    if f.author != author || f.from != from || f.to != to || f.events.len() as u64 != to - from + 1 {
        return Err(bad());
    }
    if f.events.iter().enumerate().any(|(i, e)| e.device != author || e.seq != (from + i as u64) as i64) {
        return Err(bad());
    }
    Ok(f.events)
}

fn seal_blob(key: &CloudKey, id: &str, mime: &str, data: &[u8]) -> Result<(String, Vec<u8>)> {
    let raw_id = unhex32(id).ok_or_else(|| fail(CloudFail::Format, "неверный номер обложки"))?;
    if mime.is_empty() || mime.len() > 255 {
        return Err(fail(CloudFail::Format, "неверный тип обложки"));
    }
    let mut plain = Vec::with_capacity(1 + mime.len() + 32 + data.len());
    plain.push(mime.len() as u8);
    plain.extend_from_slice(mime.as_bytes());
    plain.extend_from_slice(&raw_id);
    plain.extend_from_slice(data);
    let name = key.blob_name(id);
    let sealed = seal_file(key, KIND_BLOB, &name, &plain)?;
    Ok((name, sealed))
}

/// Обложка из облака: тип и снимок, отпечаток которого совпал с ожидаемым номером.
fn open_blob(key: &CloudKey, id: &str, name: &str, data: &[u8]) -> Result<(String, Vec<u8>)> {
    let bad = || fail(CloudFail::Format, format!("обложка в облаке повреждена ({name})"));
    let plain = open_file(key, KIND_BLOB, name, data)?;
    let mime_len = *plain.first().ok_or_else(bad)? as usize;
    let mime = std::str::from_utf8(plain.get(1..1 + mime_len).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
    let raw_id = plain.get(1 + mime_len..1 + mime_len + 32).ok_or_else(bad)?;
    let body = plain[1 + mime_len + 32..].to_vec();
    if hex(raw_id) != id || db::blob_id(&body) != id {
        return Err(bad());
    }
    db::check_blob(&mime, &body).map_err(|_| bad())?;
    Ok((mime, body))
}

// ——— Обмен ———

/// Что устройство помнит об облаке между обменами (хранится в базе устройства).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Файлы этого устройства в облаке, сверенные с базой.
    #[serde(default)]
    pub checked: BTreeSet<String>,
    /// Обложки, которые точно есть в облаке (имена файлов).
    #[serde(default)]
    pub blobs: BTreeSet<String>,
    /// Когда список обложек в облаке сверяли целиком: секунды Unix.
    #[serde(default)]
    pub blobs_listed: i64,
    /// Обмен уже проходил хотя бы раз.
    #[serde(default)]
    pub synced: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    from: u64,
    to: u64,
}

impl Span {
    fn name(self, author: &str) -> String {
        event_name(author, self.from, self.to)
    }
}

enum Fetched {
    Events(Vec<Event>),
    /// Файл пропал между списком и скачиванием (его слило другое устройство).
    Vanished,
    Broken(String),
}

fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

/// Последний номер события каждого устройства в базе.
fn known(store: &Mutex<Store>) -> Result<BTreeMap<String, u64>> {
    Ok(lock(store).versions()?.into_iter().map(|(d, s)| (d, s.max(0) as u64)).collect())
}

/// Отрезки 1..=max, не покрытые файлами.
fn gaps(spans: &[Span], max: u64) -> Vec<(u64, u64)> {
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|s| s.from);
    let mut out = Vec::new();
    let mut next = 1u64;
    for s in sorted {
        if s.from > next {
            out.push((next, (s.from - 1).min(max)));
        }
        next = next.max(s.to.saturating_add(1));
        if next > max {
            break;
        }
    }
    if next <= max {
        out.push((next, max));
    }
    out.retain(|(a, b)| a <= b);
    out
}

/// Отрезок, нарезанный на куски по FILE_EVENTS — одинаково на всех устройствах.
fn chunks(from: u64, to: u64) -> Vec<Span> {
    let mut out = Vec::new();
    let mut a = from;
    while a <= to {
        let b = to.min(a + FILE_EVENTS - 1);
        out.push(Span { from: a, to: b });
        a = b + 1;
    }
    out
}

impl Round<'_> {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(fail(CloudFail::Cancelled, "Обмен остановлен"))
        } else {
            Ok(())
        }
    }

    /// Облако то же, к которому подключались: ключ на месте и с тем же номером.
    fn same_cloud(&self) -> Result<()> {
        match read_key_file(self.cloud)? {
            None => Err(gone()),
            Some(kf) if kf.group != self.key.group => Err(replaced()),
            Some(_) => Ok(()),
        }
    }

    fn list_events(&self) -> Result<BTreeMap<String, Vec<Span>>> {
        let mut out: BTreeMap<String, Vec<Span>> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for e in self.cloud.list(Dir::Events)? {
            if !seen.insert(e.name.clone()) {
                continue;
            }
            if let Some(f) = parse_event_name(&e.name) {
                out.entry(f.author).or_default().push(Span { from: f.from, to: f.to });
            }
        }
        Ok(out)
    }

    fn events_range(&self, author: &str, span: Span) -> Result<Vec<Event>> {
        lock(self.store).events_range(author, span.from as i64, span.to as i64)
    }

    fn fetch_once(&self, author: &str, span: Span) -> Result<Fetched> {
        let name = span.name(author);
        let data = match self.cloud.get(Dir::Events, &name, MAX_EVENTS_FILE) {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(Fetched::Vanished),
            Err(Error::Cloud(CloudFail::Format, m)) => return Ok(Fetched::Broken(m)),
            Err(e) => return Err(e),
        };
        match open_events(self.key, author, span.from, span.to, &data) {
            Ok(events) => Ok(Fetched::Events(events)),
            Err(e @ Error::Cloud(CloudFail::Newer, _)) => Err(e),
            Err(e) => Ok(Fetched::Broken(e.to_string())),
        }
    }

    /// Скачивает файл событий; не расшифровался — ещё одна попытка (вдруг файл испортился по дороге).
    fn fetch(&self, author: &str, span: Span) -> Result<Fetched> {
        match self.fetch_once(author, span)? {
            Fetched::Broken(_) => self.fetch_once(author, span),
            other => Ok(other),
        }
    }

    /// События, которые уже есть в базе, должны совпасть с облаком.
    fn check_overlap(&self, events: &[Event], have: u64) -> Result<()> {
        let st = lock(self.store);
        for ev in events.iter().filter(|e| e.seq as u64 <= have) {
            if st.event_body(&ev.device, ev.seq)?.is_some_and(|b| b != ev.body) {
                return Err(conflict());
            }
        }
        Ok(())
    }

    /// Файл не расшифровался. Если облако подключили заново, дальше обмениваться нельзя.
    /// Если ключ тот же — файл испорчен или подменён, прочитать его не сможет никто: удаляем,
    /// и устройство, у которого есть эти записи, увидит пропуск и отправит их снова.
    fn broken(&self, dir: Dir, name: &str, why: String, report: &mut Report, checked_cloud: &mut bool) -> Result<()> {
        if !*checked_cloud {
            self.same_cloud()?;
            *checked_cloud = true;
        }
        self.cloud.delete(dir, name)?;
        let source = if dir == Dir::Blobs { "где есть эта обложка" } else { "где есть эти записи" };
        report.problems.push(format!("{why}: файл удалён, его заново отправит устройство, {source}"));
        Ok(())
    }

    fn download(&self, me: &str, files: &BTreeMap<String, Vec<Span>>, bad: &mut BTreeSet<String>, state: &mut State, report: &mut Report) -> Result<()> {
        let mut vv = known(self.store)?;
        let mut checked_cloud = false;
        for (author, spans) in files {
            loop {
                self.check()?;
                let have = vv.get(author).copied().unwrap_or(0);
                // Файл, который продолжает журнал без пропуска и уходит дальше всех.
                let next = spans.iter().filter(|s| s.from <= have + 1 && s.to > have && !bad.contains(&s.name(author))).max_by_key(|s| s.to).copied();
                let Some(span) = next else { break };
                let name = span.name(author);
                match self.fetch(author, span)? {
                    Fetched::Events(events) => {
                        self.check_overlap(&events, have)?;
                        report.received += lock(self.store).apply_remote(&events)?;
                        vv.insert(author.clone(), span.to);
                        if author == me {
                            state.checked.insert(name);
                        }
                    }
                    Fetched::Vanished => {
                        bad.insert(name);
                    }
                    Fetched::Broken(why) => {
                        self.broken(Dir::Events, &name, why, report, &mut checked_cloud)?;
                        bad.insert(name);
                    }
                }
            }
        }
        // Свои файлы, которые ещё не сверяли: база этого устройства могла начаться заново под тем же номером.
        if let Some(spans) = files.get(me) {
            let have = vv.get(me).copied().unwrap_or(0);
            for span in spans {
                let name = span.name(me);
                if span.to > have || state.checked.contains(&name) || bad.contains(&name) {
                    continue;
                }
                self.check()?;
                match self.fetch(me, *span)? {
                    Fetched::Events(events) => {
                        self.check_overlap(&events, have)?;
                        state.checked.insert(name);
                    }
                    Fetched::Vanished => {
                        bad.insert(name);
                    }
                    Fetched::Broken(why) => {
                        self.broken(Dir::Events, &name, why, report, &mut checked_cloud)?;
                        bad.insert(name);
                    }
                }
            }
        }
        Ok(())
    }

    fn upload(&self, me: &str, files: &mut BTreeMap<String, Vec<Span>>, bad: &BTreeSet<String>, state: &mut State, report: &mut Report) -> Result<()> {
        let vv = known(self.store)?;
        for (author, &max) in &vv {
            let spans = files.entry(author.clone()).or_default();
            let good: Vec<Span> = spans.iter().copied().filter(|s| !bad.contains(&s.name(author))).collect();
            for (a, b) in gaps(&good, max) {
                for span in chunks(a, b) {
                    self.check()?;
                    let events = self.events_range(author, span)?;
                    if events.len() as u64 != span.to - span.from + 1 {
                        report.problems.push(format!("в журнале устройства {author} пропуск — события {}…{} не отправлены", span.from, span.to));
                        break;
                    }
                    let (name, data) = seal_events(self.key, author, span.from, span.to, &events)?;
                    self.cloud.put(Dir::Events, &name, &data)?;
                    report.sent += events.len();
                    if !spans.contains(&span) {
                        spans.push(span);
                    }
                    if author == me {
                        state.checked.insert(name);
                    }
                }
            }
        }
        Ok(())
    }

    /// Сливает мелкие файлы устройства, если их накопилось много. Сливается только то, что есть в базе.
    fn compact(&self, me: &str, files: &mut BTreeMap<String, Vec<Span>>, bad: &BTreeSet<String>, state: &mut State) -> Result<()> {
        let vv = known(self.store)?;
        for (author, spans) in files.iter_mut() {
            let mut good: Vec<Span> = spans.iter().copied().filter(|s| !bad.contains(&s.name(author))).collect();
            if good.len() <= COMPACT_AFTER {
                continue;
            }
            let have = vv.get(author).copied().unwrap_or(0);
            good.retain(|s| s.to <= have);
            good.sort_by_key(|s| (s.from, std::cmp::Reverse(s.to)));
            let mut runs: Vec<Vec<Span>> = Vec::new();
            for s in good {
                match runs.last_mut() {
                    Some(run) if s.from <= run.iter().map(|x| x.to).max().unwrap_or(0) + 1 => run.push(s),
                    _ => runs.push(vec![s]),
                }
            }
            'run: for run in runs {
                let end = run.iter().map(|s| s.to).max().unwrap_or(0);
                let targets = chunks(run[0].from, end);
                if targets.len() >= run.len() {
                    continue;
                }
                for t in &targets {
                    if run.contains(t) {
                        continue;
                    }
                    self.check()?;
                    let events = self.events_range(author, *t)?;
                    if events.len() as u64 != t.to - t.from + 1 {
                        continue 'run;
                    }
                    let (name, data) = seal_events(self.key, author, t.from, t.to, &events)?;
                    self.cloud.put(Dir::Events, &name, &data)?;
                    if author == me {
                        state.checked.insert(name);
                    }
                }
                for s in &run {
                    if !targets.contains(s) {
                        self.check()?;
                        self.cloud.delete(Dir::Events, &s.name(author))?;
                    }
                }
                spans.retain(|s| !run.contains(s) || targets.contains(s));
                for t in targets {
                    if !spans.contains(&t) {
                        spans.push(t);
                    }
                }
            }
        }
        Ok(())
    }

    fn blobs(&self, state: &mut State, report: &mut Report, progress: &mut dyn FnMut(Step)) -> Result<()> {
        let (present, missing) = {
            let st = lock(self.store);
            (st.present_blobs()?, st.missing_blobs()?)
        };
        let named = |ids: Vec<String>| -> Vec<(String, String)> { ids.into_iter().map(|id| (self.key.blob_name(&id), id)).collect() };
        let (present, missing) = (named(present), named(missing));
        if present.is_empty() && missing.is_empty() {
            return Ok(());
        }
        let unknown = present.iter().chain(missing.iter()).any(|(n, _)| !state.blobs.contains(n));
        if unknown || self.now - state.blobs_listed >= BLOBS_RELIST {
            self.check()?;
            state.blobs = self.cloud.list(Dir::Blobs)?.into_iter().map(|e| e.name).collect();
            state.blobs_listed = self.now;
        }
        let up: Vec<&(String, String)> = present.iter().filter(|(n, _)| !state.blobs.contains(n)).collect();
        for (i, (name, id)) in up.iter().enumerate() {
            self.check()?;
            progress(Step::Upload { done: i, total: up.len() });
            let Some((mime, data)) = lock(self.store).blob(id)? else { continue };
            let (_, sealed) = seal_blob(self.key, id, &mime, &data)?;
            self.cloud.put(Dir::Blobs, name, &sealed)?;
            state.blobs.insert(name.clone());
            report.files_sent += 1;
        }
        let down: Vec<&(String, String)> = missing.iter().filter(|(n, _)| state.blobs.contains(n)).collect();
        report.files_waiting = missing.len() - down.len();
        let mut checked_cloud = false;
        for (i, (name, id)) in down.iter().enumerate() {
            self.check()?;
            progress(Step::Download { done: i, total: down.len() });
            let mut opened = None;
            for _ in 0..2 {
                opened = match self.cloud.get(Dir::Blobs, name, MAX_BLOB_FILE) {
                    Ok(None) => None,
                    Ok(Some(data)) => Some(open_blob(self.key, id, name, &data)),
                    Err(e @ Error::Cloud(CloudFail::Format, _)) => Some(Err(e)),
                    Err(e) => return Err(e),
                };
                if !matches!(opened, Some(Err(_))) {
                    break;
                }
            }
            match opened {
                None => {
                    state.blobs.remove(name);
                    report.files_waiting += 1;
                }
                Some(Ok((mime, body))) => {
                    lock(self.store).put_blob_checked(id, &mime, &body)?;
                    report.files_received += 1;
                }
                Some(Err(e @ Error::Cloud(CloudFail::Newer, _))) => return Err(e),
                Some(Err(e)) => {
                    self.broken(Dir::Blobs, name, e.to_string(), report, &mut checked_cloud)?;
                    state.blobs.remove(name);
                    report.files_waiting += 1;
                }
            }
        }
        Ok(())
    }

    /// Файлы книг: отправить те, которых в облаке нет целиком, и забрать нужные — когда в облаке
    /// лежат все их куски. Недокачанный файл докачивается с того же куска в следующий раз.
    fn books(&self, report: &mut Report, progress: &mut dyn FnMut(Step)) -> Result<()> {
        let (present, wanted) = {
            let st = lock(self.store);
            if !files::sync_on(&st) {
                return Ok(());
            }
            (files::present(&st)?, files::wanted(&st)?)
        };
        if present.is_empty() && wanted.is_empty() {
            return Ok(());
        }
        self.check()?;
        let names: BTreeSet<String> = self.cloud.list(Dir::Books)?.into_iter().map(|e| e.name).collect();
        let parts_of = |f: &BookFile| -> Vec<String> { (0..f.parts()).map(|i| self.key.book_name(&f.id, i, f.parts())).collect() };
        let up: Vec<(&BookFile, Vec<String>)> = present.iter().map(|f| (f, parts_of(f))).filter(|(_, p)| p.iter().any(|n| !names.contains(n))).collect();
        let down: Vec<(&BookFile, Vec<String>)> = wanted.iter().map(|f| (f, parts_of(f))).filter(|(_, p)| p.iter().all(|n| names.contains(n))).collect();
        report.books_waiting = wanted.len() - down.len();
        let total = up.len() + down.len();
        for (i, (f, parts)) in up.iter().enumerate() {
            progress(Step::Books { done: i, total });
            let mut whole = true;
            for (k, name) in parts.iter().enumerate() {
                if names.contains(name) {
                    continue;
                }
                self.check()?;
                let Some((_, data)) = files::read_part(&lock(self.store), &f.id, k as u64 * files::PART as u64)? else {
                    whole = false;
                    break;
                };
                let sealed = seal_file(self.key, KIND_BOOK, name, &data)?;
                self.cloud.put(Dir::Books, name, &sealed)?;
            }
            if whole {
                report.books_sent += 1;
            }
        }
        for (i, (f, parts)) in down.iter().enumerate() {
            progress(Step::Books { done: up.len() + i, total });
            let have = files::partial(&lock(self.store), &f.id);
            let start = if have < f.size && have.is_multiple_of(files::PART as u64) { have / files::PART as u64 } else { 0 };
            for (k, name) in parts.iter().enumerate().skip(start as usize) {
                self.check()?;
                // Кусок пропал между списком и скачиванием — в другой раз.
                let Some(data) = self.cloud.get(Dir::Books, name, MAX_BOOK_FILE)? else { break };
                let plain = match open_file(self.key, KIND_BOOK, name, &data) {
                    Ok(p) => p,
                    Err(e @ Error::Cloud(CloudFail::Newer, _)) => return Err(e),
                    Err(e) => {
                        report.problems.push(format!("Файл книги «{}»: {e}", f.name));
                        break;
                    }
                };
                match files::write_part(&lock(self.store), f, k as u64 * files::PART as u64, f.size, &plain) {
                    Ok(true) => {
                        report.books_received += 1;
                        break;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        report.problems.push(format!("Файл книги «{}»: {e}", f.name));
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Обмен с облаком: забрать новое, отправить недостающее, слить мелкие файлы, обложки.
/// `state` меняется по ходу: сохраните его и при ошибке — сделанное не повторится.
pub fn sync(r: &Round, state: &mut State, progress: &mut dyn FnMut(Step)) -> Result<Report> {
    let mut report = Report::default();
    progress(Step::Events);
    r.check()?;
    if !r.cloud.exists(Dir::Root, KEY_FILE)? {
        return Err(gone());
    }
    let me = lock(r.store).device().to_string();
    let mut files = r.list_events()?;
    if files.is_empty() && state.synced {
        // Файлы пропали: облако очистили или подключили заново. Если ключ тот же — отправим всё снова.
        r.same_cloud()?;
    }
    let mut bad = BTreeSet::new();
    r.download(&me, &files, &mut bad, state, &mut report)?;
    r.upload(&me, &mut files, &bad, state, &mut report)?;
    r.compact(&me, &mut files, &bad, state)?;
    let names: BTreeSet<String> = files.iter().flat_map(|(a, spans)| spans.iter().map(move |s| s.name(a))).collect();
    state.checked.retain(|n| names.contains(n));
    r.blobs(state, &mut report, progress)?;
    r.books(&mut report, progress)?;
    state.synced = true;
    Ok(report)
}

// ——— Облако в памяти: для проверок ———

/// Облако в памяти — для проверок обмена без сети.
#[derive(Default)]
pub struct MemCloud {
    files: Mutex<BTreeMap<(Dir, String), Vec<u8>>>,
    /// Сколько было обращений.
    pub calls: AtomicUsize,
}

impl MemCloud {
    fn files(&self) -> MutexGuard<'_, BTreeMap<(Dir, String), Vec<u8>>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn names(&self, dir: Dir) -> Vec<String> {
        self.files().keys().filter(|(d, _)| *d == dir).map(|(_, n)| n.clone()).collect()
    }

    pub fn raw(&self, dir: Dir, name: &str) -> Option<Vec<u8>> {
        self.files().get(&(dir, name.to_string())).cloned()
    }

    pub fn set_raw(&self, dir: Dir, name: &str, data: Vec<u8>) {
        self.files().insert((dir, name.to_string()), data);
    }

    pub fn remove(&self, dir: Dir, name: &str) {
        self.files().remove(&(dir, name.to_string()));
    }

    fn call(&self) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

impl Cloud for MemCloud {
    fn list(&self, dir: Dir) -> Result<Vec<Entry>> {
        self.call();
        Ok(self.files().iter().filter(|((d, _), _)| *d == dir).map(|((_, n), v)| Entry { name: n.clone(), size: v.len() as u64 }).collect())
    }

    fn get(&self, dir: Dir, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        self.call();
        match self.files().get(&(dir, name.to_string())) {
            Some(v) if v.len() > max => Err(fail(CloudFail::Format, format!("файл в облаке больше {max} байт ({name})"))),
            other => Ok(other.cloned()),
        }
    }

    fn put(&self, dir: Dir, name: &str, data: &[u8]) -> Result<()> {
        self.call();
        self.files().insert((dir, name.to_string()), data.to_vec());
        Ok(())
    }

    fn exists(&self, dir: Dir, name: &str) -> Result<bool> {
        self.call();
        Ok(self.files().contains_key(&(dir, name.to_string())))
    }

    fn delete(&self, dir: Dir, name: &str) -> Result<()> {
        self.call();
        self.files().remove(&(dir, name.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Title, KIND};
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
        assert!(cloud.names(Dir::Books).is_empty());
        lock(&a).set_meta(SYNC_KEY, "1").unwrap();
        let r = round(&cloud, &a, &key, &mut sa).unwrap();
        assert_eq!(r.books_sent, 1);
        let names = cloud.names(Dir::Books);
        assert_eq!(names.len(), 3, "три куска: {names:?}");
        assert!(names.iter().all(|n| ours(Dir::Books, n) && !n.contains(&id[..16])));
        // Второе устройство: записи пришли, файла нет, пока обмен файлами выключен.
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!(r.books_received, 0);
        lock(&b).set_meta(SYNC_KEY, "1").unwrap();
        // Пока в облаке не все куски — ждём.
        let last = key.book_name(&id, 2, 3);
        let kept = cloud.raw(Dir::Books, &last).unwrap();
        cloud.remove(Dir::Books, &last);
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_waiting), (0, 1));
        // Подменённый кусок не пройдёт: имя входит в проверку подлинности.
        cloud.set_raw(Dir::Books, &last, cloud.raw(Dir::Books, &key.book_name(&id, 1, 3)).unwrap());
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!(r.books_received, 0);
        assert!(!r.problems.is_empty());
        cloud.set_raw(Dir::Books, &last, kept);
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_waiting), (1, 0), "{r:?}");
        let got = crate::files::path_in(lock(&b).books_dir().unwrap(), &id).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), data);
        // Больше делать нечего.
        let r = round(&cloud, &b, &key, &mut sb).unwrap();
        assert_eq!((r.books_received, r.books_sent), (0, 0));
        // Удаление данных из облака убирает и куски файлов.
        wipe(&cloud, &AtomicBool::new(false)).unwrap();
        assert!(cloud.names(Dir::Books).is_empty());
    }

    #[test]
    fn key_file_opens_only_with_the_password() {
        let mk = CloudKey::generate().unwrap();
        let kf = KeyFile::parse(&mk.key_file(PW, FAST, 1).unwrap().to_bytes()).unwrap();
        let again = CloudKey::from_key_file(&kf, PW).unwrap();
        assert_eq!((again.key, again.group()), (mk.key, mk.group()));
        assert!(matches!(CloudKey::from_key_file(&kf, "отпуск в Сочи!"), Err(Error::CloudPassword)));
        // Подменить номер облака или параметры незаметно нельзя.
        let mut other = kf.clone();
        other.group = "0".repeat(32);
        assert!(matches!(CloudKey::from_key_file(&other, PW), Err(Error::CloudPassword)));
        let mut weaker = kf.clone();
        weaker.t = 2;
        assert!(matches!(CloudKey::from_key_file(&weaker, PW), Err(Error::CloudPassword)));
        // Огромные параметры из чужого файла не считаются.
        let mut greedy = kf.clone();
        greedy.m = 4 << 20;
        assert!(matches!(CloudKey::from_key_file(&greedy, PW), Err(Error::Cloud(CloudFail::Format, _))));
        assert!(matches!(KeyFile::parse(b"{}"), Err(Error::Cloud(CloudFail::Format, _))));
        let mut newer = kf.clone();
        newer.format = 9;
        assert!(matches!(KeyFile::parse(&newer.to_bytes()), Err(Error::Cloud(CloudFail::Newer, _))));
        assert!(matches!(mk.key_file("1234567", FAST, 1), Err(Error::Backup(m)) if m.contains("не короче 8")));
        // Ключ на устройстве.
        let back = CloudKey::from_meta(&mk.to_meta()).unwrap();
        assert_eq!((back.key, back.group()), (mk.key, mk.group()));
        assert!(CloudKey::from_meta("1:abc:xyz").is_err());
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
    fn tampered_renamed_and_foreign_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let cloud = MemCloud::default();
        let a = demo(dir.path(), "a.db", "aaaa");
        let (k, _) = join(&cloud, PW, FAST, 1).unwrap();
        let mut sa = State::default();
        round(&cloud, &a, &k, &mut sa).unwrap();
        let name = cloud.names(Dir::Events)[0].clone();
        let span = parse_event_name(&name).unwrap();
        // Изменённый байт.
        let mut bad = cloud.raw(Dir::Events, &name).unwrap();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        cloud.set_raw(Dir::Events, &name, bad);
        let total = lock(&a).event_count().unwrap() as usize;
        let b = fresh(dir.path(), "b.db", "bbbb");
        let mut sb = State::default();
        let r = round(&cloud, &b, &k, &mut sb).unwrap();
        assert_eq!(r.received, 0);
        assert!(r.problems.iter().any(|p| p.contains("повреждён") && p.contains("удалён")), "{:?}", r.problems);
        assert!(cloud.raw(Dir::Events, &name).is_none(), "испорченный файл удалён");
        // Устройство A видит пропуск и отправляет записи заново — и всё сходится.
        assert_eq!(round(&cloud, &a, &k, &mut sa).unwrap().sent, total);
        let r = round(&cloud, &b, &k, &mut sb).unwrap();
        assert!(r.received == total && r.problems.is_empty(), "{r:?}");
        // Тот же файл под другим именем (будто в нём больше событий) не расшифруется.
        let good = cloud.raw(Dir::Events, &name).unwrap();
        let fake = event_name(&span.author, span.from, span.to + 100);
        cloud.set_raw(Dir::Events, &fake, good);
        let c = fresh(dir.path(), "c.db", "cccc");
        let mut sc = State::default();
        let r = round(&cloud, &c, &k, &mut sc).unwrap();
        assert!(r.problems.iter().any(|p| p.contains(&fake)), "{:?}", r.problems);
        assert!(cloud.raw(Dir::Events, &fake).is_none());
        assert_eq!(versions(&c).get("aaaa"), versions(&a).get("aaaa"));
        // Обложка, подменённая другим снимком, не принимается и удаляется; A отправит её снова.
        let blob = cloud.names(Dir::Blobs)[0].clone();
        let (_, other) = seal_blob(&k, &db::blob_id(&jpeg(10, 1)), "image/jpeg", &jpeg(10, 1)).unwrap();
        cloud.set_raw(Dir::Blobs, &blob, other);
        let d = fresh(dir.path(), "d.db", "dddd");
        let mut sd = State::default();
        let r = round(&cloud, &d, &k, &mut sd).unwrap();
        assert_eq!((r.files_received, r.files_waiting), (0, 1));
        assert!(r.problems.iter().any(|p| p.contains("обложка")), "{:?}", r.problems);
        assert!(cloud.raw(Dir::Blobs, &blob).is_none());
        sa.blobs_listed = 0; // раз в сутки A сверяет список обложек в облаке
        assert_eq!(round(&cloud, &a, &k, &mut sa).unwrap().files_sent, 1);
        assert_eq!(round(&cloud, &d, &k, &mut sd).unwrap().files_received, 1);
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
        assert_eq!(k4.key, k3.key);
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

    #[test]
    fn gaps_and_chunks() {
        let s = |from, to| Span { from, to };
        assert_eq!(gaps(&[], 3), vec![(1, 3)]);
        assert_eq!(gaps(&[s(1, 3)], 3), vec![]);
        assert_eq!(gaps(&[s(1, 3), s(6, 8)], 10), vec![(4, 5), (9, 10)]);
        assert_eq!(gaps(&[s(2, 9), s(1, 4)], 9), vec![]);
        assert_eq!(gaps(&[s(5, 20)], 3), vec![(1, 3)]);
        assert_eq!(chunks(1, 12_345), vec![s(1, 5000), s(5001, 10_000), s(10_001, 12_345)]);
    }
}

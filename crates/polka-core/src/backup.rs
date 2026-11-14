//! Резервная копия: весь журнал событий и обложки в одном файле, зашифрованном паролем копии.
//! Формат тот же, что у «Кармана» (своя метка в начале файла).
//!
//! Ключ базы хранится в защищённом хранилище устройства и с копией не переносится,
//! поэтому у копии свой ключ — Argon2id от пароля копии. Им зашифрован случайный ключ файла,
//! а содержимое идёт кусками по 1 МБ (STREAM на XChaCha20-Poly1305): порча, подмена,
//! перестановка и обрезка любого куска видны при чтении.
//!
//! Файл: заголовок (130 байт: магия, формат, параметры ключа, соль, зашифрованный ключ файла)
//! и куски. Внутри кусков — записи: сведения о копии, пачки событий, вложения, файлы книг
//! (с 0.8.0, если они включены в копию; кусками по 8 МБ) и итог. Старые версии записи о файлах
//! книг пропускают.
//! Восстановление добавляет события к базе так же, как синхронизация: повторы
//! пропускаются, при споре побеждает более позднее изменение.

use crate::db::{Event, Store};
use crate::files::{self, BookFile};
use crate::error::{Error, Result};
use crate::model::{Title, KIND};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"POLKABAK";
pub const FORMAT: u16 = 1;
/// Расширение файла копии.
pub const EXT: &str = "polka";
/// Самый короткий пароль копии.
pub const MIN_PASSWORD: usize = 8;

/// Кусок открытого текста.
const CHUNK: usize = 1 << 20;
/// Начало заголовка: магия, формат, вид ключа, параметры, соль.
const PREFIX: usize = 8 + 2 + 1 + 12 + 16;
/// Заголовок целиком: ещё nonce и зашифрованный ключ файла, nonce кусков.
const HEADER: usize = PREFIX + 24 + 48 + 19;
/// Старший бит длины куска — «последний кусок».
const LAST: u32 = 1 << 31;
const TAG: usize = 16;
/// Предел одной записи (обложка — до 8 МБ, пачка событий — меньше).
const MAX_RECORD: usize = 32 << 20;
/// Событий в одной записи.
const BATCH: usize = 500;

const REC_MANIFEST: u8 = 1;
const REC_EVENTS: u8 = 2;
const REC_BLOB: u8 = 3;
/// Кусок файла книги (с 0.8.0).
const REC_BOOK: u8 = 4;
const REC_END: u8 = 0xFF;

pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| Error::Random(e.to_string()))?;
    Ok(b)
}

/// Затирает ключ в памяти.
pub(crate) fn wipe(b: &mut [u8]) {
    b.fill(0);
    std::hint::black_box(b);
}

fn broken(what: &str) -> Error {
    Error::Backup(format!("Копия повреждена: {what}"))
}

/// Параметры Argon2id: память (КиБ), проходы, потоки.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kdf {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Kdf {
    /// 64 МБ и 3 прохода: на телефоне около секунды, перебор паролей — дорогой.
    pub const DEFAULT: Kdf = Kdf { m_kib: 64 * 1024, t: 3, p: 1 };

    /// Чужой или испорченный файл не должен заставить считать ключ часами или съесть всю память.
    pub(crate) fn valid(self) -> bool {
        (8 * 1024..=256 * 1024).contains(&self.m_kib) && (1..=10).contains(&self.t) && (1..=4).contains(&self.p)
    }

    fn check(self) -> Result<()> {
        if self.valid() {
            Ok(())
        } else {
            Err(Error::Backup("Копия не открывается: неподходящие параметры ключа".into()))
        }
    }
}

/// Argon2id от пароля — общий для копий и облака. Параметры должны быть проверены (`Kdf::valid`).
pub(crate) fn derive_raw(password: &str, salt: &[u8; 16], kdf: Kdf) -> Result<[u8; 32]> {
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32)).map_err(|e| Error::Backup(e.to_string()))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| Error::Backup(e.to_string()))?;
    Ok(key)
}

/// Пароль копии: не короче восьми символов.
pub fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(Error::Backup(format!("Пароль копии — не короче {MIN_PASSWORD} символов")));
    }
    Ok(())
}

/// Частые пароли и слова-пароли (буквы без цифр и знаков): их перебирают первыми.
const COMMON: &[&str] = &[
    "password", "passw", "pass", "parol", "gfhjkm", "пароль", "qwerty", "qwertyuiop", "йцукен", "polka", "полка", "admin", "letmein", "iloveyou", "welcome",
    "1q2w3e4r", "1q2w3e4r5t", "zaq12wsx", "1qaz2wsx", "привет", "privet", "любовь", "sunshine", "dragon", "monkey",
];
/// Ряды клавиш и символов подряд: «12345678», «qwertyui», «йцукенгш», «abcdefgh».
const RUNS: &[&str] = &[
    "01234567890123456789",
    "abcdefghijklmnopqrstuvwxyz",
    "qwertyuiopasdfghjklzxcvbnm",
    "1qaz2wsx3edc4rfv5tgb6yhn",
    "йцукенгшщзхъфывапролджэячсмитьбю",
    "абвгдеёжзийклмнопрстуфхцчшщъыьэюя",
];

/// Самые простые пароли: подбираются за минуты, даже с Argon2id. Возвращает, чем плох пароль.
pub fn weak_password(password: &str) -> Option<&'static str> {
    let s: String = password.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let chars: Vec<char> = s.chars().collect();
    let distinct = chars.iter().collect::<std::collections::BTreeSet<_>>().len();
    if distinct < 4 {
        return Some("в нём почти одни и те же символы");
    }
    if chars.iter().all(|c| c.is_ascii_digit()) && chars.len() < 12 {
        return Some("одни цифры подбираются быстро");
    }
    for k in 1..=4 {
        if chars.len() > k && chars.len().is_multiple_of(k) && chars.chunks(k).all(|c| c == &chars[..k]) {
            return Some("это повтор нескольких символов");
        }
    }
    let rev: String = s.chars().rev().collect();
    if RUNS.iter().any(|r| r.contains(s.as_str()) || r.contains(rev.as_str())) {
        return Some("это ряд клавиш или символов подряд");
    }
    let letters: String = s.chars().filter(|c| !c.is_ascii_digit() && !c.is_ascii_punctuation()).collect();
    if COMMON.iter().any(|w| s == *w || letters == *w) {
        return Some("это одно из самых частых слов-паролей");
    }
    None
}

/// Новый пароль копий: не короче восьми символов и не из самых простых — им шифруются копии
/// и данные в облаке, а файлы на Диске может скачать любой, кто войдёт в аккаунт.
pub fn check_new_password(password: &str) -> Result<()> {
    check_password(password)?;
    match weak_password(password) {
        Some(why) => Err(Error::Backup(format!("Слишком простой пароль: {why}. Им шифруются копии и данные в облаке — лучше фраза из нескольких слов"))),
        None => Ok(()),
    }
}

/// Ключ копий: Argon2id от пароля копии. Хранится в базе (она сама зашифрована), чтобы
/// автокопии делались без пароля; сам пароль нигде не хранится.
pub struct BackupKey {
    salt: [u8; 16],
    kdf: Kdf,
    key: [u8; 32],
}

impl Drop for BackupKey {
    fn drop(&mut self) {
        wipe(&mut self.key);
    }
}

impl BackupKey {
    /// Новый ключ из пароля, со случайной солью.
    pub fn new(password: &str) -> Result<BackupKey> {
        BackupKey::new_with(password, Kdf::DEFAULT)
    }

    /// То же с другими параметрами (в проверках — побыстрее).
    pub fn new_with(password: &str, kdf: Kdf) -> Result<BackupKey> {
        check_password(password)?;
        BackupKey::derive(password, random()?, kdf)
    }

    pub fn derive(password: &str, salt: [u8; 16], kdf: Kdf) -> Result<BackupKey> {
        kdf.check()?;
        Ok(BackupKey { salt, kdf, key: derive_raw(password, &salt, kdf)? })
    }

    /// Этот ли пароль у ключа копий: тот же пароль с той же солью даёт тот же ключ.
    pub fn matches(&self, password: &str) -> Result<bool> {
        let mut k = derive_raw(password, &self.salt, self.kdf)?;
        let same = k.iter().zip(self.key.iter()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
        wipe(&mut k);
        Ok(same)
    }

    /// Для хранения в базе: «1:память:проходы:потоки:соль:ключ».
    pub fn to_meta(&self) -> String {
        format!("1:{}:{}:{}:{}:{}", self.kdf.m_kib, self.kdf.t, self.kdf.p, B64.encode(self.salt), B64.encode(self.key))
    }

    pub fn from_meta(s: &str) -> Result<BackupKey> {
        let bad = || Error::Backup("Ключ копий повреждён: задайте пароль копии заново".into());
        let p: Vec<&str> = s.split(':').collect();
        if p.len() != 6 || p[0] != "1" {
            return Err(bad());
        }
        let num = |x: &str| x.parse::<u32>().map_err(|_| bad());
        let kdf = Kdf { m_kib: num(p[1])?, t: num(p[2])?, p: num(p[3])? };
        let salt = <[u8; 16]>::try_from(B64.decode(p[4]).map_err(|_| bad())?.as_slice()).map_err(|_| bad())?;
        let mut raw = B64.decode(p[5]).map_err(|_| bad())?;
        let key = <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| bad());
        wipe(&mut raw);
        Ok(BackupKey { salt, kdf, key: key? })
    }
}

/// Сведения о копии — первая запись. По ним видно, что восстанавливается.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u16,
    /// Версия «Полки», которая сделала копию.
    pub app: String,
    /// Когда сделана: секунды Unix.
    pub created: i64,
    /// Устройство: имя и номер.
    pub device: String,
    pub device_id: String,
    pub events: u64,
    pub blobs: u64,
    /// Записей о книгах, фильмах и сериалах.
    pub titles: u64,
    /// Файлов книг в копии (с 0.8.0).
    #[serde(default)]
    pub books: u64,
    /// Поля из более новых версий.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Кто и когда делает копию.
pub struct MadeBy {
    pub app: String,
    pub created: i64,
    pub device: String,
    pub device_id: String,
}

impl Manifest {
    pub fn new(by: &MadeBy, events: u64, blobs: u64, titles: u64) -> Manifest {
        Manifest {
            format: FORMAT,
            app: by.app.clone(),
            created: by.created,
            device: by.device.clone(),
            device_id: by.device_id.clone(),
            events,
            blobs,
            titles,
            books: 0,
            extra: Default::default(),
        }
    }
}

/// Последняя запись: сколько событий и вложений в копии на самом деле.
#[derive(Serialize, Deserialize)]
struct End {
    events: u64,
    blobs: u64,
    /// Кусков файлов книг.
    #[serde(default)]
    books: u64,
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

fn header_prefix(kdf: Kdf, salt: &[u8; 16]) -> [u8; PREFIX] {
    let mut h = [0u8; PREFIX];
    h[..8].copy_from_slice(MAGIC);
    h[8..10].copy_from_slice(&FORMAT.to_le_bytes());
    h[10] = 1; // Argon2id
    h[11..15].copy_from_slice(&kdf.m_kib.to_le_bytes());
    h[15..19].copy_from_slice(&kdf.t.to_le_bytes());
    h[19..23].copy_from_slice(&kdf.p.to_le_bytes());
    h[23..39].copy_from_slice(salt);
    h
}

/// Файл копии пишется по частям: события, затем вложения по одному — в памяти не больше куска.
pub struct Writer<W: Write> {
    out: W,
    enc: Option<EncryptorBE32<XChaCha20Poly1305>>,
    header: [u8; HEADER],
    buf: Vec<u8>,
    bytes: u64,
    events: u64,
    blobs: u64,
    /// Кусков файлов книг и самих файлов.
    book_parts: u64,
    books: u64,
}

/// Заголовок куска файла книги в копии.
#[derive(Serialize, Deserialize)]
struct BookPart {
    file: BookFile,
    offset: u64,
}

impl<W: Write> Writer<W> {
    pub fn new(mut out: W, key: &BackupKey, manifest: &Manifest) -> Result<Writer<W>> {
        let mut dek: [u8; 32] = random()?;
        let wrap_nonce: [u8; 24] = random()?;
        let stream_nonce: [u8; 19] = random()?;
        let prefix = header_prefix(key.kdf, &key.salt);
        let wrapped = XChaCha20Poly1305::new((&key.key).into())
            .encrypt(XNonce::from_slice(&wrap_nonce), Payload { msg: &dek, aad: &prefix })
            .map_err(|_| Error::Backup("не удалось зашифровать ключ файла".into()))?;
        let mut header = [0u8; HEADER];
        header[..PREFIX].copy_from_slice(&prefix);
        header[PREFIX..PREFIX + 24].copy_from_slice(&wrap_nonce);
        header[PREFIX + 24..PREFIX + 72].copy_from_slice(&wrapped);
        header[PREFIX + 72..].copy_from_slice(&stream_nonce);
        out.write_all(&header)?;
        let enc = EncryptorBE32::from_aead(XChaCha20Poly1305::new((&dek).into()), stream_nonce.as_ref().into());
        wipe(&mut dek);
        let mut w = Writer { out, enc: Some(enc), header, buf: Vec::with_capacity(CHUNK + 64), bytes: HEADER as u64, events: 0, blobs: 0, book_parts: 0, books: 0 };
        w.record(REC_MANIFEST, &serde_json::to_vec(manifest)?)?;
        Ok(w)
    }

    fn frame(&mut self, ct: &[u8], last: bool) -> Result<()> {
        let len = ct.len() as u32 | if last { LAST } else { 0 };
        self.out.write_all(&len.to_le_bytes())?;
        self.out.write_all(ct)?;
        self.bytes += 4 + ct.len() as u64;
        Ok(())
    }

    fn put(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend_from_slice(data);
        while self.buf.len() >= CHUNK {
            let enc = self.enc.as_mut().ok_or_else(|| Error::Backup("копия уже закрыта".into()))?;
            let ct = enc
                .encrypt_next(Payload { msg: &self.buf[..CHUNK], aad: &self.header })
                .map_err(|_| Error::Backup("не удалось зашифровать копию".into()))?;
            self.buf.drain(..CHUNK);
            self.frame(&ct, false)?;
        }
        Ok(())
    }

    fn record(&mut self, kind: u8, payload: &[u8]) -> Result<()> {
        if payload.len() > MAX_RECORD {
            return Err(Error::Backup("слишком большая запись для копии".into()));
        }
        self.put(&[kind])?;
        self.put(&(payload.len() as u32).to_le_bytes())?;
        self.put(payload)
    }

    pub fn events(&mut self, events: &[Event]) -> Result<()> {
        for batch in events.chunks(BATCH) {
            self.record(REC_EVENTS, &serde_json::to_vec(batch)?)?;
            self.events += batch.len() as u64;
        }
        Ok(())
    }

    pub fn blob(&mut self, id: &str, mime: &str, data: &[u8]) -> Result<()> {
        if id.len() > 255 || mime.len() > 255 {
            return Err(Error::Backup("неверное вложение".into()));
        }
        let mut p = Vec::with_capacity(2 + id.len() + mime.len() + data.len());
        p.push(id.len() as u8);
        p.extend_from_slice(id.as_bytes());
        p.push(mime.len() as u8);
        p.extend_from_slice(mime.as_bytes());
        p.extend_from_slice(data);
        self.record(REC_BLOB, &p)?;
        self.blobs += 1;
        Ok(())
    }

    /// Кусок файла книги: заголовок (длина и JSON) и данные.
    pub fn book_part(&mut self, file: &BookFile, offset: u64, data: &[u8]) -> Result<()> {
        let head = serde_json::to_vec(&BookPart { file: file.clone(), offset })?;
        let mut p = Vec::with_capacity(4 + head.len() + data.len());
        p.extend_from_slice(&(head.len() as u32).to_le_bytes());
        p.extend_from_slice(&head);
        p.extend_from_slice(data);
        self.record(REC_BOOK, &p)?;
        self.book_parts += 1;
        if offset + data.len() as u64 >= file.size {
            self.books += 1;
        }
        Ok(())
    }

    /// Итог и последний кусок. Возвращает, сколько записано, и куда писали.
    pub fn finish(mut self) -> Result<(Made, W)> {
        let end = serde_json::to_vec(&End { events: self.events, blobs: self.blobs, books: self.book_parts })?;
        self.record(REC_END, &end)?;
        let enc = self.enc.take().ok_or_else(|| Error::Backup("копия уже закрыта".into()))?;
        let ct = enc
            .encrypt_last(Payload { msg: &self.buf, aad: &self.header })
            .map_err(|_| Error::Backup("не удалось зашифровать копию".into()))?;
        self.buf.clear();
        self.frame(&ct, true)?;
        self.out.flush()?;
        Ok((Made { events: self.events, blobs: self.blobs, books: self.books, bytes: self.bytes }, self.out))
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
    input: R,
    dec: Option<DecryptorBE32<XChaCha20Poly1305>>,
    header: [u8; HEADER],
    plain: Vec<u8>,
    pos: usize,
    /// Последний кусок расшифрован.
    last: bool,
    /// Итоговая запись прочитана и сошлась.
    ended: bool,
    events: u64,
    blobs: u64,
    book_parts: u64,
    books: u64,
    pub manifest: Manifest,
}

/// Файл начинается как копия «Полки».
pub fn is_backup(path: &Path) -> bool {
    let mut head = [0u8; 8];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == MAGIC
}

fn read_header<R: Read>(input: &mut R) -> Result<([u8; HEADER], Kdf, [u8; 16])> {
    let mut header = [0u8; HEADER];
    let mut got = 0;
    while got < HEADER {
        let n = input.read(&mut header[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    if got < 8 || &header[..8] != MAGIC {
        return Err(Error::Backup("Это не резервная копия «Полки»".into()));
    }
    if got < HEADER {
        return Err(broken("файл обрывается"));
    }
    let format = u16::from_le_bytes([header[8], header[9]]);
    if format > FORMAT {
        return Err(Error::Backup("Копия сделана в более новой версии «Полки» — обновите приложение".into()));
    }
    if format == 0 || header[10] != 1 {
        return Err(broken("неизвестный формат"));
    }
    let num = |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let kdf = Kdf { m_kib: num(11), t: num(15), p: num(19) };
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&header[23..39]);
    Ok((header, kdf, salt))
}

impl<R: Read> Reader<R> {
    /// Открывает копию паролем (ключ считается заново — около секунды).
    pub fn open(mut input: R, password: &str) -> Result<Reader<R>> {
        let (header, kdf, salt) = read_header(&mut input)?;
        kdf.check()?;
        let key = BackupKey::derive(password, salt, kdf)?;
        Reader::start(input, header, &key)
    }

    /// Открывает копию сохранённым ключом — проверка только что записанной копии.
    pub fn open_with_key(mut input: R, key: &BackupKey) -> Result<Reader<R>> {
        let (header, kdf, salt) = read_header(&mut input)?;
        if kdf != key.kdf || salt != key.salt {
            return Err(Error::BackupPassword);
        }
        Reader::start(input, header, key)
    }

    fn start(input: R, header: [u8; HEADER], key: &BackupKey) -> Result<Reader<R>> {
        let mut dek = XChaCha20Poly1305::new((&key.key).into())
            .decrypt(XNonce::from_slice(&header[PREFIX..PREFIX + 24]), Payload { msg: &header[PREFIX + 24..PREFIX + 72], aad: &header[..PREFIX] })
            .map_err(|_| Error::BackupPassword)?;
        let dek_arr = <[u8; 32]>::try_from(dek.as_slice()).map_err(|_| Error::BackupPassword);
        wipe(&mut dek);
        let mut dek_arr = dek_arr?;
        let dec = DecryptorBE32::from_aead(XChaCha20Poly1305::new((&dek_arr).into()), header[PREFIX + 72..].into());
        wipe(&mut dek_arr);
        let empty = Manifest::new(&MadeBy { app: String::new(), created: 0, device: String::new(), device_id: String::new() }, 0, 0, 0);
        let mut r = Reader { input, dec: Some(dec), header, plain: Vec::new(), pos: 0, last: false, ended: false, events: 0, blobs: 0, book_parts: 0, books: 0, manifest: empty };
        let (kind, payload) = r.record()?;
        if kind != REC_MANIFEST {
            return Err(broken("нет сведений о копии"));
        }
        r.manifest = serde_json::from_slice(&payload).map_err(|_| broken("сведения о копии не читаются"))?;
        Ok(r)
    }

    /// Следующий кусок. `false` — копия кончилась.
    fn next_chunk(&mut self) -> Result<bool> {
        if self.last {
            return Ok(false);
        }
        let mut len = [0u8; 4];
        let mut got = 0;
        while got < 4 {
            let n = self.input.read(&mut len[got..])?;
            if n == 0 {
                return Err(broken("файл обрывается — копия не дописана или обрезана"));
            }
            got += n;
        }
        let len = u32::from_le_bytes(len);
        let last = len & LAST != 0;
        let n = (len & !LAST) as usize;
        if !(TAG..=CHUNK + TAG).contains(&n) {
            return Err(broken("неверная длина куска"));
        }
        let mut ct = vec![0u8; n];
        self.input.read_exact(&mut ct).map_err(|_| broken("файл обрывается — копия не дописана или обрезана"))?;
        let payload = Payload { msg: &ct, aad: &self.header };
        self.plain = if last {
            self.last = true;
            let dec = self.dec.take().ok_or_else(|| broken("лишний кусок"))?;
            dec.decrypt_last(payload).map_err(|_| broken("кусок изменён или переставлен"))?
        } else {
            let dec = self.dec.as_mut().ok_or_else(|| broken("лишний кусок"))?;
            dec.decrypt_next(payload).map_err(|_| broken("кусок изменён или переставлен"))?
        };
        self.pos = 0;
        if last {
            // После последнего куска в файле ничего быть не должно.
            let mut extra = [0u8; 1];
            if self.input.read(&mut extra)? != 0 {
                return Err(broken("после конца копии лишние данные"));
            }
        }
        Ok(true)
    }

    fn take(&mut self, out: &mut [u8]) -> Result<()> {
        let mut got = 0;
        while got < out.len() {
            if self.pos == self.plain.len() && !self.next_chunk()? {
                return Err(broken("копия кончается посреди записи"));
            }
            let n = (out.len() - got).min(self.plain.len() - self.pos);
            out[got..got + n].copy_from_slice(&self.plain[self.pos..self.pos + n]);
            self.pos += n;
            got += n;
        }
        Ok(())
    }

    fn record(&mut self) -> Result<(u8, Vec<u8>)> {
        let mut head = [0u8; 5];
        self.take(&mut head)?;
        let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > MAX_RECORD {
            return Err(broken("слишком большая запись"));
        }
        let mut payload = vec![0u8; len];
        self.take(&mut payload)?;
        Ok((head[0], payload))
    }

    /// Следующая запись: пачка событий или вложение. `None` — копия прочитана целиком и сошлась.
    pub fn next_item(&mut self) -> Result<Option<Item>> {
        loop {
            if self.ended {
                return Ok(None);
            }
            let (kind, payload) = self.record()?;
            match kind {
                REC_EVENTS => {
                    let events: Vec<Event> = serde_json::from_slice(&payload).map_err(|_| broken("события не читаются"))?;
                    self.events += events.len() as u64;
                    return Ok(Some(Item::Events(events)));
                }
                REC_BLOB => {
                    let bad = || broken("вложение не читается");
                    let id_len = *payload.first().ok_or_else(bad)? as usize;
                    let id = std::str::from_utf8(payload.get(1..1 + id_len).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
                    let mime_len = *payload.get(1 + id_len).ok_or_else(bad)? as usize;
                    let at = 2 + id_len;
                    let mime = std::str::from_utf8(payload.get(at..at + mime_len).ok_or_else(bad)?).map_err(|_| bad())?.to_string();
                    let data = payload[at + mime_len..].to_vec();
                    self.blobs += 1;
                    return Ok(Some(Item::Blob { id, mime, data }));
                }
                REC_BOOK => {
                    let bad = || broken("файл книги не читается");
                    let len = u32::from_le_bytes(payload.get(..4).ok_or_else(bad)?.try_into().map_err(|_| bad())?) as usize;
                    let head: BookPart = serde_json::from_slice(payload.get(4..4 + len).ok_or_else(bad)?).map_err(|_| bad())?;
                    let data = payload[4 + len..].to_vec();
                    self.book_parts += 1;
                    if head.offset + data.len() as u64 >= head.file.size {
                        self.books += 1;
                    }
                    return Ok(Some(Item::Book { file: head.file, offset: head.offset, data }));
                }
                REC_END => {
                    let end: End = serde_json::from_slice(&payload).map_err(|_| broken("итог не читается"))?;
                    if end.events != self.events || end.blobs != self.blobs || end.books != self.book_parts {
                        return Err(broken("не хватает записей"));
                    }
                    // Итог — последняя запись последнего куска.
                    if self.pos != self.plain.len() || !self.last {
                        return Err(broken("после итога ещё данные"));
                    }
                    self.ended = true;
                    return Ok(None);
                }
                REC_MANIFEST => return Err(broken("сведения о копии повторяются")),
                // Записи из более новой версии того же формата пропускаются.
                _ => continue,
            }
        }
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
    let mut manifest = Manifest::new(by, events.len() as u64, blobs.len() as u64, store.list::<Title>(KIND)?.len() as u64);
    manifest.books = books.len() as u64;
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
        let big = s.put_blob("image/jpeg", &jpeg(CHUNK + 300_000, 9)).unwrap();
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
        assert_eq!(checked.manifest.titles, 3);
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
        assert_eq!((checked.books, checked.manifest.books), (1, 1));
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
    fn wrong_password_tampering_and_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let src = demo_store(dir.path(), "a.db", "dev-a");
        let key = BackupKey::new_with("книги на даче", FAST).unwrap();
        let file = backup_of(&src, &key);
        let check = |data: &[u8], pw: &str| -> Result<Checked> { verify(Reader::open(Cursor::new(data), pw)?, "x", |_| Ok(false)) };

        assert!(matches!(check(&file, "книги на даче!"), Err(Error::BackupPassword)));
        // Изменённый байт в середине — кусок не расшифруется.
        let mut bad = file.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        assert!(matches!(check(&bad, "книги на даче"), Err(Error::Backup(m)) if m.contains("повреждена")));
        // Подменённые параметры ключа в заголовке — ключ файла не расшифруется.
        let mut bad = file.clone();
        bad[15] = 2;
        assert!(matches!(check(&bad, "книги на даче"), Err(Error::BackupPassword)));
        // Обрезанный файл и лишние байты в конце.
        assert!(matches!(check(&file[..file.len() - 10], "книги на даче"), Err(Error::Backup(m)) if m.contains("обрывается")));
        let mut tail = file.clone();
        tail.push(0);
        assert!(matches!(check(&tail, "книги на даче"), Err(Error::Backup(m)) if m.contains("лишние")));
        // Переставленные куски.
        let first_len = (u32::from_le_bytes(file[HEADER..HEADER + 4].try_into().unwrap()) & !LAST) as usize;
        assert_eq!(first_len, CHUNK + TAG, "копия больше куска — кусков несколько");
        let a = HEADER..HEADER + 4 + first_len;
        let b_len = (u32::from_le_bytes(file[a.end..a.end + 4].try_into().unwrap()) & !LAST) as usize;
        let mut swapped = file[..HEADER].to_vec();
        swapped.extend_from_slice(&file[a.end..a.end + 4 + b_len]);
        swapped.extend_from_slice(&file[a.clone()]);
        swapped.extend_from_slice(&file[a.end + 4 + b_len..]);
        assert!(check(&swapped, "книги на даче").is_err());
        // Не копия вовсе, копия «Кармана» и копия из будущей версии.
        assert!(matches!(check(b"PK\x03\x04 not a backup", "книги на даче"), Err(Error::Backup(m)) if m.contains("не резервная копия")));
        let mut karman = file.clone();
        karman[..8].copy_from_slice(b"KARMANBK");
        assert!(matches!(check(&karman, "книги на даче"), Err(Error::Backup(m)) if m.contains("«Полки»")));
        let mut newer = file.clone();
        newer[8] = 9;
        assert!(matches!(check(&newer, "книги на даче"), Err(Error::Backup(m)) if m.contains("более новой версии")));
        // Огромные параметры ключа из чужого файла не принимаются.
        let mut greedy = file.clone();
        greedy[11..15].copy_from_slice(&(4u32 << 20).to_le_bytes());
        assert!(matches!(check(&greedy, "книги на даче"), Err(Error::Backup(m)) if m.contains("параметры")));
    }

    #[test]
    fn key_meta_and_password_rules() {
        assert!(matches!(BackupKey::new_with("1234567", FAST), Err(Error::Backup(m)) if m.contains("не короче 8")));
        for weak in ["12345678", "87654321", "qwertyui", "йцукенгш", "abcdefgh", "aaaaaaaa", "abababab", "Пароль2026", "password123!", "1q2w3e4r", "полка 123", "11112222"] {
            assert!(matches!(check_new_password(weak), Err(Error::Backup(m)) if m.contains("Слишком простой")), "{weak}");
        }
        for fine in ["книги на даче 2026", "моя домашняя полка", "correct horse battery", "Кот Барсик читает", "759301846215"] {
            assert!(check_new_password(fine).is_ok(), "{fine}");
        }
        let key = BackupKey::new_with("длинный пароль", FAST).unwrap();
        let again = BackupKey::from_meta(&key.to_meta()).unwrap();
        assert_eq!((again.key, again.salt, again.kdf), (key.key, key.salt, key.kdf));
        assert!(BackupKey::from_meta("1:2:3").is_err());
        assert!(key.matches("длинный пароль").unwrap() && !key.matches("другой пароль").unwrap());
        // Сохранённым ключом копия открывается без пароля; другая соль — другой ключ.
        let dir = tempfile::tempdir().unwrap();
        let src = demo_store(dir.path(), "a.db", "dev-a");
        let file = backup_of(&src, &key);
        let checked = verify(Reader::open_with_key(Cursor::new(&file), &again).unwrap(), "x", |_| Ok(false)).unwrap();
        assert_eq!(checked.blobs, 2);
        let other = BackupKey::new_with("длинный пароль", FAST).unwrap();
        assert!(matches!(Reader::open_with_key(Cursor::new(&file), &other), Err(Error::BackupPassword)));
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

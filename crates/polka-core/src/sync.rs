//! Обмен записями между устройствами — общий для всех каналов (Wi-Fi, позже — облако).
//!
//! Устроен как в «Кармане»: каждое устройство знает последний номер события от каждого устройства
//! (вектор версий). При обмене стороны называют свои векторы, отдают друг другу недостающие
//! события и потом — недостающие снимки обложек (по номерам из записей). Повторы пропускаются,
//! при споре побеждает более позднее изменение (это делает база).
//!
//! С 0.8.0 после обложек — файлы книг (если обмен ими включён на обоих устройствах): каждая сторона
//! забирает у другой недостающие файлы кусками, с докачкой (см. [`crate::files`]).
//!
//! Канал ([`Wire`]) шифрует каждое сообщение; ключи и порядок пакетов — в [`crate::lan`].

use crate::db::{Event, Store};
use crate::files::{self, BookFile};
use crate::error::{Error, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::TcpStream;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Последний известный номер события от каждого устройства.
pub type VersionVector = BTreeMap<String, i64>;

/// Версия «Полки»: другое устройство узнаёт её при обмене.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Самое большое сообщение обмена.
pub(crate) const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Первое сообщение (приветствие) маленькое: до проверки ключа не принимаем больше мегабайта
/// и ждём его недолго — чужой в сети не займёт приём подключений.
pub(crate) const HELLO_MAX: usize = 1024 * 1024;
const HELLO_WAIT: Duration = Duration::from_secs(15);
/// Столько байт обложек в одном пакете — с запасом до предела кадра.
const FILES_PER_PACKET: usize = 24 * 1024 * 1024;
/// Больше кругов обмена обложками за одно подключение не делаем.
const MAX_ROUNDS: usize = 50;
/// Файлов книг за одно подключение — не больше (остальные — в следующий раз).
const MAX_FILES: usize = 100;
/// Кусков файлов за одно подключение — с запасом на 100 файлов по 100 МБ.
const MAX_PARTS: usize = MAX_FILES * 14 + 16;

/// Сообщения обмена.
#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(crate) enum Msg {
    /// Кто я — сразу после связи по коду.
    Ident { device: String, name: String, app: String },
    Hello { device: String, name: String, vv: VersionVector, app: String },
    Batch { device: String, name: String, vv: VersionVector, events: Vec<Event>, app: String },
    Done { applied: usize },
    /// Каких обложек не хватает.
    Want { ids: Vec<String> },
    /// Обложки по запросу и свой список недостающих.
    Give { blobs: Vec<Blob>, want: Vec<String> },
    /// Файлы книг (с 0.8.0): готово ли устройство отдавать свои.
    Files { give: bool },
    /// Кусок файла книги с места `offset`.
    Pull { id: String, offset: u64 },
    /// Кусок файла книги: `total` — размер файла (0 — такого файла нет), данные — base64.
    Part { id: String, offset: u64, total: u64, data: String },
    /// Больше кусков не нужно.
    FilesDone,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Blob {
    id: String,
    mime: String,
    /// Содержимое в base64.
    data: String,
}

/// Чем закончился обмен.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SyncReport {
    pub peer: String,
    pub peer_name: String,
    /// Сколько событий ушло другому устройству и было им принято.
    pub sent: usize,
    /// Сколько новых событий пришло к нам.
    pub received: usize,
    /// Сколько обложек ушло и пришло.
    pub files_sent: usize,
    pub files_received: usize,
    /// Версия «Полки» на другом устройстве.
    pub peer_app: String,
    /// Сколько файлов книг ушло и пришло целиком.
    #[serde(default)]
    pub books_sent: usize,
    #[serde(default)]
    pub books_received: usize,
}

/// Файлы книг по Wi-Fi — с 0.8.0: со старой «Полкой» эту часть обмена пропускаем.
pub(crate) fn speaks_files(app: &str) -> bool {
    let mut it = app.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    let (major, minor) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    major > 0 || minor >= 8
}

/// Забрать у другой стороны недостающие файлы — кусками, с докачкой. Возвращает, сколько пришло целиком.
fn pull_files(w: &mut impl Wire, store: &Mutex<Store>, list: &[BookFile]) -> Result<usize> {
    let mut got = 0;
    for f in list.iter().take(MAX_FILES) {
        let mut offset = files::partial(&lock(store), &f.id);
        if offset >= f.size {
            offset = 0;
        }
        loop {
            w.send(&Msg::Pull { id: f.id.clone(), offset })?;
            let (id, at, total, data) = match w.recv()? {
                Msg::Part { id, offset, total, data } => (id, offset, total, data),
                _ => return Err(unexpected()),
            };
            if id != f.id || at != offset || total == 0 {
                break;
            }
            let Ok(bytes) = B64.decode(data.as_bytes()) else { break };
            match files::write_part(&lock(store), f, offset, total, &bytes) {
                Ok(true) => {
                    got += 1;
                    break;
                }
                Ok(false) => offset += bytes.len() as u64,
                // Испорченный или не тот файл — в другой раз.
                Err(_) => break,
            }
        }
    }
    Ok(got)
}

/// Отдавать куски по запросам другой стороны, пока она не скажет «хватит».
/// Возвращает, сколько файлов отдано целиком.
fn serve_files(w: &mut impl Wire, store: &Mutex<Store>, give: bool) -> Result<usize> {
    let known: std::collections::BTreeSet<String> = if give { files::present(&lock(store))?.into_iter().map(|f| f.id).collect() } else { Default::default() };
    let mut sent = 0;
    for _ in 0..MAX_PARTS {
        match w.recv()? {
            Msg::Pull { id, offset } => {
                let part = if known.contains(&id) { files::read_part(&lock(store), &id, offset)? } else { None };
                match part {
                    Some((total, data)) if !data.is_empty() => {
                        if offset + data.len() as u64 >= total {
                            sent += 1;
                        }
                        w.send(&Msg::Part { id, offset, total, data: B64.encode(&data) })?;
                    }
                    _ => w.send(&Msg::Part { id, offset, total: 0, data: String::new() })?,
                }
            }
            Msg::FilesDone => return Ok(sent),
            _ => return Err(unexpected()),
        }
    }
    Err(unexpected())
}

/// Канал обмена: зашифрованные сообщения поверх TCP.
pub(crate) trait Wire {
    fn send(&mut self, msg: &Msg) -> Result<()>;
    /// Следующее сообщение, не больше `max` байт.
    fn recv_max(&mut self, max: usize) -> Result<Msg>;
    fn stream(&self) -> &TcpStream;
    fn recv(&mut self) -> Result<Msg> {
        self.recv_max(MAX_FRAME)
    }
}

fn lock(store: &Mutex<Store>) -> MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn prepare(s: &TcpStream) -> Result<()> {
    // Пакет с обложками по медленному Wi-Fi идёт долго.
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    s.set_write_timeout(Some(Duration::from_secs(60)))?;
    s.set_nodelay(true)?;
    Ok(())
}

fn unexpected() -> Error {
    Error::Format("неожиданный ответ другого устройства".into())
}

/// Достаёт запрошенные обложки, пока не наберётся предел пакета.
/// Возвращает обложки и сколько из имеющихся не поместилось.
fn collect_blobs(store: &Mutex<Store>, ids: &[String]) -> Result<(Vec<Blob>, usize)> {
    let st = lock(store);
    let (mut out, mut total, mut left) = (Vec::new(), 0usize, 0usize);
    for id in ids {
        if let Some((mime, data)) = st.blob(id)? {
            if total + data.len() > FILES_PER_PACKET && !out.is_empty() {
                left += 1;
                continue;
            }
            total += data.len();
            out.push(Blob { id: id.clone(), mime, data: B64.encode(&data) });
        }
    }
    Ok((out, left))
}

/// Сохраняет полученные обложки: только запрошенные и только с верным отпечатком.
fn store_blobs(store: &Mutex<Store>, asked: &[String], blobs: Vec<Blob>) -> usize {
    let mut st = lock(store);
    let mut n = 0;
    for b in blobs {
        if !asked.contains(&b.id) {
            continue;
        }
        let Ok(data) = B64.decode(b.data.as_bytes()) else { continue };
        if st.put_blob_checked(&b.id, &b.mime, &data).is_ok() {
            n += 1;
        }
    }
    n
}

/// Обмен со стороны подключившегося. `expect` — какое устройство должно ответить
/// (по ключу пары): ответ другого — обрыв до того, как его записи приняты.
pub(crate) fn run_client(w: &mut impl Wire, store: &Mutex<Store>, name: &str, expect: Option<&str>) -> Result<SyncReport> {
    prepare(w.stream())?;
    let (device, vv) = {
        let st = lock(store);
        (st.device().to_string(), st.versions()?)
    };
    w.send(&Msg::Hello { device: device.clone(), name: name.into(), vv, app: APP_VERSION.into() })?;
    let (peer, peer_name, their_vv, events, peer_app) = match w.recv()? {
        Msg::Batch { device, name, vv, events, app } => (device, name, vv, events, app),
        _ => return Err(unexpected()),
    };
    if expect.is_some_and(|e| e != peer) {
        return Err(Error::Format("ответило не то устройство".into()));
    }
    let (received, ours) = {
        let mut st = lock(store);
        let n = st.apply_remote(&events)?;
        (n, st.events_missing(&their_vv)?)
    };
    let vv = lock(store).versions()?;
    w.send(&Msg::Batch { device, name: name.into(), vv, events: ours, app: APP_VERSION.into() })?;
    let sent = match w.recv()? {
        Msg::Done { applied } => applied,
        _ => return Err(unexpected()),
    };
    let (mut files_sent, mut files_received) = (0, 0);
    for _ in 0..MAX_ROUNDS {
        let want = lock(store).missing_blobs()?;
        w.send(&Msg::Want { ids: want.clone() })?;
        let (blobs, their_want) = match w.recv()? {
            Msg::Give { blobs, want } => (blobs, want),
            _ => return Err(unexpected()),
        };
        let got = store_blobs(store, &want, blobs);
        files_received += got;
        let (mine, left) = collect_blobs(store, &their_want)?;
        w.send(&Msg::Give { blobs: mine, want: Vec::new() })?;
        match w.recv()? {
            Msg::Done { applied } => files_sent += applied,
            _ => return Err(unexpected()),
        }
        let still = lock(store).missing_blobs()?.len();
        if (still == 0 || got == 0) && left == 0 {
            break;
        }
    }
    // Файлы книг: сначала забираем свои недостающие, потом отдаём то, что нужно другой стороне.
    let (mut books_sent, mut books_received) = (0, 0);
    if speaks_files(&peer_app) && files::sync_on(&lock(store)) {
        let mine = files::wanted(&lock(store))?;
        w.send(&Msg::Files { give: true })?;
        let they_give = match w.recv()? {
            Msg::Files { give } => give,
            _ => return Err(unexpected()),
        };
        if they_give {
            books_received = pull_files(w, store, &mine)?;
        }
        w.send(&Msg::FilesDone)?;
        books_sent = serve_files(w, store, true)?;
    }
    w.send(&Msg::Done { applied: 0 })?;
    Ok(SyncReport { peer, peer_name, sent, received, files_sent, files_received, peer_app, books_sent, books_received })
}

/// Обмен со стороны ждущего. `expect` — какое устройство должно прийти
/// (по ключу пары): другое имя в приветствии — обрыв.
pub(crate) fn run_server(w: &mut impl Wire, store: &Mutex<Store>, name: &str, expect: Option<&str>) -> Result<SyncReport> {
    prepare(w.stream())?;
    w.stream().set_read_timeout(Some(HELLO_WAIT))?;
    let (peer, peer_name, their_vv, peer_app) = match w.recv_max(HELLO_MAX)? {
        Msg::Hello { device, name, vv, app } => (device, name, vv, app),
        _ => return Err(Error::Format("ожидалось приветствие".into())),
    };
    if expect.is_some_and(|e| e != peer) {
        return Err(Error::Format("приветствие не от того устройства".into()));
    }
    // Ключ подошёл — дальше пакеты с обложками, их можно ждать дольше.
    prepare(w.stream())?;
    let (device, vv, events) = {
        let st = lock(store);
        (st.device().to_string(), st.versions()?, st.events_missing(&their_vv)?)
    };
    let sent = events.len();
    w.send(&Msg::Batch { device, name: name.into(), vv, events, app: APP_VERSION.into() })?;
    let incoming = match w.recv()? {
        Msg::Batch { events, .. } => events,
        _ => return Err(Error::Format("ожидались события".into())),
    };
    let received = lock(store).apply_remote(&incoming)?;
    w.send(&Msg::Done { applied: received })?;
    let (mut files_sent, mut files_received) = (0, 0);
    let (mut books_sent, mut books_received) = (0, 0);
    for _ in 0..MAX_ROUNDS {
        match w.recv()? {
            Msg::Files { give: they_give } => {
                let on = files::sync_on(&lock(store));
                let mine = if on { files::wanted(&lock(store))? } else { Vec::new() };
                w.send(&Msg::Files { give: on })?;
                books_sent += serve_files(w, store, on)?;
                if on && they_give {
                    books_received += pull_files(w, store, &mine)?;
                }
                w.send(&Msg::FilesDone)?;
            }
            Msg::Want { ids } => {
                let (blobs, _) = collect_blobs(store, &ids)?;
                files_sent += blobs.len();
                let want = lock(store).missing_blobs()?;
                w.send(&Msg::Give { blobs, want: want.clone() })?;
                let incoming = match w.recv()? {
                    Msg::Give { blobs, .. } => blobs,
                    _ => return Err(Error::Format("ожидались обложки".into())),
                };
                let n = store_blobs(store, &want, incoming);
                files_received += n;
                w.send(&Msg::Done { applied: n })?;
            }
            Msg::Done { .. } => break,
            _ => return Err(unexpected()),
        }
    }
    Ok(SyncReport { peer, peer_name, sent, received, files_sent, files_received, peer_app, books_sent, books_received })
}

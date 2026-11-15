//! Связь по Wi-Fi: поиск устройств в сети, связь по коду и обмен со связанными устройствами.
//! Устроено как в «Кармане» 0.15 — со своими портами и метками, чтобы приложения не мешали друг другу.
//!
//! **Связь по коду.** Одно устройство показывает код из восьми знаков ([`crate::pairing::new_code`]),
//! другое его вводит. Из кода оба получают общий ключ по SPAKE2: сам код по сети не идёт, а тот, кто
//! подслушал обмен, не может проверить у себя ни одного варианта кода. Каждая попытка — только
//! подключением к устройству с кодом; после пяти неверных код отменяется. Ключ подтверждают обе
//! стороны, и только потом идёт что-то ещё: подставное устройство, не знающее кода, ничего не получит.
//!
//! **Ключ пары.** Из ключа SPAKE2 оба выводят ключ пары и запоминают его вместе с номером другого
//! устройства. Дальше обмен — по этому ключу: у каждого подключения свои случайные числа обеих сторон,
//! ключи сеанса — свои для каждого направления, у каждого пакета — свой номер. Повтор старого
//! подключения, перестановка и подмена пакетов не проходят.
//!
//! **Поиск в сети.** Запрос — широковещательный пакет UDP. Отвечают только устройство с показанным
//! кодом (имя и порт — чтобы выбрать его в списке) и связанные устройства: им запрос подписан ключом
//! пары, и ответ тоже. Чужим в сети — в гостинице, в кафе — «Полка» не отвечает.

use crate::db::Store;
use crate::error::{Error, LanFail, Result};
use crate::pairing::normalize_code;
use crate::sync::{prepare, run_client, run_server, Msg, SyncReport, Wire, APP_VERSION, MAX_FRAME};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use blake2::digest::consts::U32;
use blake2::digest::Mac;
use blake2::Blake2bMac;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::Serialize;
use serde_json::{json, Value};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Порт обмена (TCP). У «Кармана» — 47500, у «Полки» свой.
pub const TCP_PORT: u16 = 47510;
/// Порт поиска в сети (UDP).
pub const DISCOVERY_PORT: u16 = 47511;
/// Код действует пять минут.
pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);
/// Неверных попыток до отмены кода.
pub const CODE_TRIES: u8 = 5;

const MAGIC: &[u8; 4] = b"PLKL";
const Q_MAGIC: &[u8; 4] = b"PLKQ";
const A_MAGIC: &[u8; 4] = b"PLKA";
const VERSION: u8 = 1;
/// Открытые сообщения (до ключа) маленькие и ждём их недолго.
const PLAIN_MAX: usize = 4096;
const PLAIN_WAIT: Duration = Duration::from_secs(10);
/// Подключение к устройству в сети.
const CONNECT_WAIT: Duration = Duration::from_secs(4);
/// Одновременных подключений к серверу не больше стольких — остальные закрываются сразу.
const MAX_CONNECTIONS: usize = 4;
/// Запросов поиска в секунду разбираем не больше стольких — остальные отбрасываем не глядя.
const MAX_QUERIES: usize = 60;
const ID_JOINER: &[u8] = b"polka-joiner";
const ID_HOST: &[u8] = b"polka-host";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// MAC и вывод ключей: BLAKE2b с ключом, части — с длиной, чтобы границы не сдвигались.
fn mac(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Blake2bMac::<U32>::new_with_salt_and_personal(key, &[], b"polka-lan").expect("ключ MAC — 32 байта");
    for p in parts {
        Mac::update(&mut m, &(p.len() as u32).to_be_bytes());
        Mac::update(&mut m, p);
    }
    m.finalize().into_bytes().into()
}

/// Сравнение без утечки по времени.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| Error::Random(e.to_string()))?;
    Ok(b)
}

fn b64(v: &[u8]) -> String {
    B64.encode(v)
}

/// Поле base64 ровно нужной длины.
fn field<const N: usize>(v: &Value, name: &str) -> Result<[u8; N]> {
    let raw = v[name].as_str().and_then(|t| B64.decode(t).ok()).ok_or_else(|| Error::Format(format!("нет поля {name}")))?;
    raw.try_into().map_err(|_| Error::Format(format!("поле {name} неверной длины")))
}

/// Номер устройства: латиница, цифры, «-» и «_», до 64 знаков.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Имя устройства с другой стороны: без управляющих знаков, до 64 знаков.
fn clean_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect::<String>().trim().to_string()
}

// ——— Открытые сообщения до ключа ———

fn write_plain(s: &mut TcpStream, kind: u8, body: &Value) -> Result<()> {
    let b = serde_json::to_vec(body)?;
    if b.len() > PLAIN_MAX {
        return Err(Error::Format("слишком большой запрос".into()));
    }
    let mut out = Vec::with_capacity(8 + b.len());
    out.extend_from_slice(MAGIC);
    out.push(kind);
    out.push(VERSION);
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(&b);
    s.write_all(&out)?;
    s.flush()?;
    Ok(())
}

fn read_plain(s: &mut TcpStream) -> Result<(u8, Value)> {
    let mut head = [0u8; 8];
    s.read_exact(&mut head)?;
    if &head[..4] != MAGIC {
        return Err(Error::Format("это не «Полка»".into()));
    }
    if head[5] != VERSION {
        return Err(Error::Peer("На другом устройстве более новая версия «Полки»: обновите это устройство".into()));
    }
    let len = u16::from_be_bytes([head[6], head[7]]) as usize;
    if len > PLAIN_MAX {
        return Err(Error::Format("слишком большой запрос".into()));
    }
    let mut b = vec![0u8; len];
    s.read_exact(&mut b)?;
    Ok((head[4], serde_json::from_slice(&b)?))
}

// ——— Сеанс: свои ключи на каждое направление и номер у каждого пакета ———

/// Зашифрованный канал сеанса: XChaCha20-Poly1305, nonce — номер пакета. Ключи сеанса не повторяются,
/// поэтому номера с нуля безопасны; пакет не на своём месте не расшифруется.
pub(crate) struct SessionWire {
    s: TcpStream,
    tx: XChaCha20Poly1305,
    rx: XChaCha20Poly1305,
    tx_n: u64,
    rx_n: u64,
}

fn nonce(n: u64) -> XNonce {
    let mut b = [0u8; 24];
    b[..8].copy_from_slice(&n.to_le_bytes());
    XNonce::from(b)
}

impl SessionWire {
    fn new(s: TcpStream, tx: [u8; 32], rx: [u8; 32]) -> SessionWire {
        SessionWire { s, tx: XChaCha20Poly1305::new(&tx.into()), rx: XChaCha20Poly1305::new(&rx.into()), tx_n: 0, rx_n: 0 }
    }
}

impl Wire for SessionWire {
    fn send(&mut self, msg: &Msg) -> Result<()> {
        let plain = serde_json::to_vec(msg)?;
        let ct = self.tx.encrypt(&nonce(self.tx_n), Payload { msg: &plain, aad: MAGIC }).map_err(|_| Error::Decrypt)?;
        self.tx_n += 1;
        self.s.write_all(&(ct.len() as u32).to_be_bytes())?;
        self.s.write_all(&ct)?;
        self.s.flush()?;
        Ok(())
    }

    fn recv_max(&mut self, max: usize) -> Result<Msg> {
        let mut len = [0u8; 4];
        self.s.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > max.min(MAX_FRAME) {
            return Err(Error::Format("слишком большой пакет".into()));
        }
        // Память — по мере прихода данных, а не сразу на объявленную длину: подставной не займёт её словом.
        let mut buf = Vec::new();
        (&mut self.s).take(len as u64).read_to_end(&mut buf)?;
        if buf.len() != len {
            return Err(Error::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        let plain = self.rx.decrypt(&nonce(self.rx_n), Payload { msg: &buf, aad: MAGIC }).map_err(|_| Error::Decrypt)?;
        self.rx_n += 1;
        Ok(serde_json::from_slice(&plain)?)
    }

    fn stream(&self) -> &TcpStream {
        &self.s
    }
}

// ——— Код связи ———

/// Показанный код связи.
#[derive(Clone, Debug)]
pub struct CodeState {
    /// Восемь знаков, без дефиса.
    pub code: String,
    pub until: Instant,
    /// Сколько попыток осталось; 0 — код отменён.
    pub left: u8,
    /// Сколько неверных попыток уже было.
    pub wrong: u8,
}

impl CodeState {
    pub fn new(code: String) -> CodeState {
        CodeState { code, until: Instant::now() + CODE_TTL, left: CODE_TRIES, wrong: 0 }
    }

    /// Код ещё можно ввести.
    pub fn active(&self) -> bool {
        self.left > 0 && Instant::now() < self.until
    }
}

/// Устройство с другой стороны.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Peer {
    pub device: String,
    pub name: String,
    pub app: String,
}

type KeyFn = dyn Fn(&str) -> Option<[u8; 32]> + Send + Sync;
type PairedFn = dyn Fn(&Peer, &[u8; 32]) -> std::result::Result<(), String> + Send + Sync;
type SyncedFn = dyn Fn(&str, &std::result::Result<SyncReport, String>) + Send + Sync;
/// Что сделать, когда оба подтвердили код: запомнить устройство и ключ пары.
pub type OnPaired<'a> = dyn Fn(&Peer, &[u8; 32]) -> std::result::Result<(), String> + 'a;

/// Это устройство в сети: чем отвечать и куда записывать. Общее у сервера и ответчика на поиск.
#[derive(Clone)]
pub struct Host {
    pub store: Arc<Mutex<Store>>,
    pub device: String,
    pub name: String,
    /// Показанный код; None — кода нет.
    pub code: Arc<Mutex<Option<CodeState>>>,
    /// Ключ пары по номеру связанного устройства.
    pub peer_key: Arc<KeyFn>,
    /// Связались по коду — запомнить устройство и ключ (до первого обмена). Ошибка — обрыв без обмена.
    pub paired: Arc<PairedFn>,
    /// Обмен закончился — с кем и чем.
    pub synced: Arc<SyncedFn>,
}

impl Host {
    fn code_active(&self) -> bool {
        lock(&self.code).as_ref().is_some_and(CodeState::active)
    }

    /// Одно подключение: связь по коду или обмен со связанным устройством.
    pub fn handle(&self, mut s: TcpStream) -> Result<()> {
        s.set_nodelay(true)?;
        s.set_read_timeout(Some(PLAIN_WAIT))?;
        s.set_write_timeout(Some(PLAIN_WAIT))?;
        let (kind, body) = read_plain(&mut s)?;
        match kind {
            b'P' => self.host_pair(s, &body),
            b'S' => self.host_sync(s, &body),
            _ => Err(Error::Format("неизвестный запрос".into())),
        }
    }

    fn host_pair(&self, mut s: TcpStream, body: &Value) -> Result<()> {
        let a: [u8; 33] = field(body, "a")?;
        // Каждая попытка — минус одна, ещё до проверки: подключение, оборванное после ответа, — тоже попытка.
        let attempt = {
            let mut c = lock(&self.code);
            match c.as_mut() {
                Some(st) if st.active() => {
                    st.left -= 1;
                    Ok((st.code.clone(), st.left))
                }
                Some(st) if st.left == 0 => Err(LanFail::Cancelled),
                _ => Err(LanFail::NoCode),
            }
        };
        let (code, left) = match attempt {
            Ok(x) => x,
            Err(f) => {
                write_plain(&mut s, b'P', &json!({ "err": if f == LanFail::Cancelled { "cancelled" } else { "no_code" } }))?;
                return Err(lan_error(f, ""));
            }
        };
        let (sp, b) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &Identity::new(ID_JOINER), &Identity::new(ID_HOST));
        let k: [u8; 32] = sp.finish(&a).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| Error::Format("испорченный запрос связи".into()))?;
        write_plain(&mut s, b'P', &json!({ "b": b64(&b), "left": left }))?;
        let (_, body) = read_plain(&mut s)?;
        let confirm: [u8; 32] = field(&body, "confirm")?;
        if !same(&confirm, &mac(&k, &[b"joiner confirm"])) {
            let left = {
                let mut c = lock(&self.code);
                match c.as_mut() {
                    Some(st) => {
                        st.wrong += 1;
                        st.left
                    }
                    None => 0,
                }
            };
            write_plain(&mut s, b'P', &json!({ "err": "wrong_code", "left": left }))?;
            return Err(lan_error(LanFail::WrongCode(left), ""));
        }
        // Код верный — больше не действует.
        lock(&self.code).take();
        write_plain(&mut s, b'P', &json!({ "confirm": b64(&mac(&k, &[b"host confirm"])) }))?;
        let mut w = SessionWire::new(s, mac(&k, &[b"pair s2c"]), mac(&k, &[b"pair c2s"]));
        let peer = match w.recv_max(PLAIN_MAX)? {
            Msg::Ident { device, name, app } if valid_id(&device) && device != self.device => Peer { device, name: clean_name(&name), app },
            _ => return Err(Error::Format("устройство не назвалось".into())),
        };
        (self.paired)(&peer, &mac(&k, &[b"pair key"])).map_err(Error::Peer)?;
        w.send(&Msg::Ident { device: self.device.clone(), name: self.name.clone(), app: APP_VERSION.into() })?;
        let r = run_server(&mut w, &self.store, &self.name, Some(&peer.device)).map_err(|e| e.to_string());
        (self.synced)(&peer.device, &r);
        r.map(|_| ()).map_err(Error::Peer)
    }

    fn host_sync(&self, mut s: TcpStream, body: &Value) -> Result<()> {
        let from = body["from"].as_str().filter(|d| valid_id(d)).ok_or_else(|| Error::Format("нет номера устройства".into()))?.to_string();
        let to = body["to"].as_str().unwrap_or("");
        let nc: [u8; 16] = field(body, "nonce")?;
        if to != self.device {
            write_plain(&mut s, b'S', &json!({ "err": "other" }))?;
            return Err(lan_error(LanFail::NotPeer, ""));
        }
        let Some(key) = (self.peer_key)(&from) else {
            write_plain(&mut s, b'S', &json!({ "err": "unknown" }))?;
            return Err(lan_error(LanFail::Unknown, ""));
        };
        let ns: [u8; 16] = random()?;
        write_plain(&mut s, b'S', &json!({ "nonce": b64(&ns) }))?;
        let mut w = SessionWire::new(s, mac(&key, &[b"sync s2c", &nc, &ns]), mac(&key, &[b"sync c2s", &nc, &ns]));
        let r = run_server(&mut w, &self.store, &self.name, Some(&from)).map_err(|e| e.to_string());
        (self.synced)(&from, &r);
        r.map(|_| ()).map_err(Error::Peer)
    }
}

/// Ошибка связи с понятным текстом.
fn lan_error(f: LanFail, peer: &str) -> Error {
    let who = if peer.is_empty() { "Другое устройство".to_string() } else { format!("«{peer}»") };
    let text = match f {
        LanFail::NoCode => "На том устройстве код не показан или уже не действует: покажите код заново".to_string(),
        LanFail::WrongCode(0) => "Код не подошёл, и он отменён: пять неверных попыток. Покажите на том устройстве новый код".to_string(),
        LanFail::WrongCode(n) => format!("Код не подошёл. Осталось попыток: {n}"),
        LanFail::Cancelled => "Код отменён: было пять неверных попыток. Покажите на том устройстве новый код".to_string(),
        LanFail::Unknown => format!("{who} не узнало это устройство: возможно, там связь отвязали. Свяжите устройства заново"),
        LanFail::Busy => format!("{who} занято другим обменом — попробуйте через минуту"),
        LanFail::NotPeer => "Ответило не то устройство: оно не знает кода или ключа. Проверьте, что выбрано нужное".to_string(),
    };
    Error::Lan(f, text)
}

fn lan_reply_error(v: &Value, peer: &str) -> Option<Error> {
    let err = v["err"].as_str()?;
    let left = v["left"].as_u64().unwrap_or(0).min(CODE_TRIES as u64) as u8;
    Some(lan_error(
        match err {
            "no_code" => LanFail::NoCode,
            "cancelled" => LanFail::Cancelled,
            "wrong_code" => LanFail::WrongCode(left),
            "unknown" => LanFail::Unknown,
            "busy" => LanFail::Busy,
            _ => LanFail::NotPeer,
        },
        peer,
    ))
}

fn connect(addr: SocketAddr) -> Result<TcpStream> {
    let s = TcpStream::connect_timeout(&addr, CONNECT_WAIT)?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(PLAIN_WAIT))?;
    s.set_write_timeout(Some(PLAIN_WAIT))?;
    Ok(s)
}

/// Связь по коду с устройством по адресу: ключ пары и первый обмен. `paired` запоминает устройство
/// сразу, как оба подтвердили код, — до обмена; ошибка в нём — обрыв без обмена.
pub fn pair_join(addr: SocketAddr, code: &str, store: &Mutex<Store>, device: &str, name: &str, paired: &OnPaired) -> Result<(Peer, SyncReport)> {
    let code = normalize_code(code)?;
    let mut s = connect(addr)?;
    let (sp, a) = Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &Identity::new(ID_JOINER), &Identity::new(ID_HOST));
    write_plain(&mut s, b'P', &json!({ "a": b64(&a) }))?;
    let (_, r) = read_plain(&mut s)?;
    if let Some(e) = lan_reply_error(&r, "") {
        return Err(e);
    }
    let b: [u8; 33] = field(&r, "b")?;
    let k: [u8; 32] = sp.finish(&b).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| Error::Format("испорченный ответ на связь".into()))?;
    write_plain(&mut s, b'P', &json!({ "confirm": b64(&mac(&k, &[b"joiner confirm"])) }))?;
    let (_, r) = read_plain(&mut s)?;
    if let Some(e) = lan_reply_error(&r, "") {
        return Err(e);
    }
    let confirm: [u8; 32] = field(&r, "confirm")?;
    if !same(&confirm, &mac(&k, &[b"host confirm"])) {
        return Err(lan_error(LanFail::NotPeer, ""));
    }
    let mut w = SessionWire::new(s, mac(&k, &[b"pair c2s"]), mac(&k, &[b"pair s2c"]));
    prepare(w.stream())?;
    w.send(&Msg::Ident { device: device.into(), name: name.into(), app: APP_VERSION.into() })?;
    let peer = match w.recv_max(PLAIN_MAX)? {
        Msg::Ident { device: d, name, app } if valid_id(&d) && d != device => Peer { device: d, name: clean_name(&name), app },
        _ => return Err(Error::Format("устройство не назвалось".into())),
    };
    paired(&peer, &mac(&k, &[b"pair key"])).map_err(Error::Peer)?;
    let report = run_client(&mut w, store, name, Some(&peer.device))?;
    Ok((peer, report))
}

/// Обмен со связанным устройством по ключу пары.
pub fn sync_peer(addr: SocketAddr, me: &str, peer: &str, peer_name: &str, key: &[u8; 32], store: &Mutex<Store>, name: &str) -> Result<SyncReport> {
    let mut s = connect(addr)?;
    let nc: [u8; 16] = random()?;
    write_plain(&mut s, b'S', &json!({ "from": me, "to": peer, "nonce": b64(&nc) }))?;
    let (_, r) = read_plain(&mut s)?;
    if let Some(e) = lan_reply_error(&r, peer_name) {
        return Err(e);
    }
    let ns: [u8; 16] = field(&r, "nonce")?;
    let mut w = SessionWire::new(s, mac(key, &[b"sync c2s", &nc, &ns]), mac(key, &[b"sync s2c", &nc, &ns]));
    run_client(&mut w, store, name, Some(peer)).map_err(|e| match e {
        // Ответ не расшифровался — у того, кто ответил, нет ключа пары.
        Error::Decrypt => lan_error(LanFail::NotPeer, peer_name),
        e => e,
    })
}

// ——— Сервер ———

/// Приём подключений в фоне: связь по коду и обмен со связанными устройствами.
pub struct Server {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Слушает `port`; занят — любой свободный.
    pub fn start(host: Host, port: u16) -> Result<Server> {
        let listener = TcpListener::bind(("0.0.0.0", port)).or_else(|_| TcpListener::bind(("0.0.0.0", 0)))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let busy = Arc::new(AtomicUsize::new(0));
        let thread = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        // Лишние подключения — сразу закрыть: чужой в сети не займёт приём.
                        if busy.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            drop(s);
                            continue;
                        }
                        let _ = s.set_nonblocking(false);
                        busy.fetch_add(1, Ordering::Relaxed);
                        let (host, busy) = (host.clone(), busy.clone());
                        std::thread::spawn(move || {
                            let _ = host.handle(s);
                            busy.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(60)),
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        Ok(Server { port, stop, thread: Some(thread) })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ——— Поиск в сети ———

/// Найдено в сети: устройство с показанным кодом или связанное устройство.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    pub device: String,
    /// Имя — только у устройства с показанным кодом (связанные мы и так знаем).
    pub name: String,
    /// Куда подключаться (TCP).
    pub addr: SocketAddr,
    /// Показан код: с ним можно связаться.
    pub pairing: bool,
    /// Связанное устройство: ответ подписан ключом пары.
    pub peer: bool,
    pub app: String,
}

fn datagram(magic: &[u8; 4], body: &Value) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.push(VERSION);
    out.extend_from_slice(&serde_json::to_vec(body).unwrap_or_default());
    out
}

fn parse_datagram(magic: &[u8; 4], data: &[u8]) -> Option<Value> {
    if data.len() < 5 || data.len() > 1400 || &data[..4] != magic || data[4] != VERSION {
        return None;
    }
    serde_json::from_slice(&data[5..]).ok()
}

/// Ответ на запрос поиска — или ничего (чужому не отвечаем).
fn answer(host: &Host, tcp_port: u16, q: &Value) -> Option<Value> {
    let from = q["from"].as_str().filter(|d| valid_id(d))?;
    if from == host.device {
        return None;
    }
    match q["t"].as_str()? {
        // Устройство без связи ищет, с кем связаться: отвечаем, только пока показан код.
        "pair" if host.code_active() => Some(json!({ "t": "pair", "d": host.device, "name": host.name, "p": tcp_port, "app": APP_VERSION })),
        // Связанное устройство: запрос подписан ключом пары — и ответ тоже.
        "peer" => {
            if q["to"].as_str()? != host.device {
                return None;
            }
            let key = (host.peer_key)(from)?;
            let n: [u8; 16] = field(q, "n").ok()?;
            let m: [u8; 32] = field(q, "m").ok()?;
            if !same(&m, &mac(&key, &[b"query", from.as_bytes(), host.device.as_bytes(), &n])) {
                return None;
            }
            let port = tcp_port.to_string();
            let am = mac(&key, &[b"answer", host.device.as_bytes(), from.as_bytes(), &n, port.as_bytes()]);
            Some(json!({ "t": "peer", "d": host.device, "p": tcp_port, "app": APP_VERSION, "m": b64(&am) }))
        }
        _ => None,
    }
}

/// Отвечает на поиск в сети, пока не остановят.
pub struct Responder {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Responder {
    /// Слушает `port` (UDP); `tcp_port` — порт сервера для ответа.
    pub fn start(host: Host, port: u16, tcp_port: u16) -> Result<Responder> {
        let sock = UdpSocket::bind(("0.0.0.0", port))?;
        let port = sock.local_addr()?.port();
        sock.set_read_timeout(Some(Duration::from_millis(250)))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            let (mut second, mut seen) = (Instant::now(), 0usize);
            while !stop2.load(Ordering::Relaxed) {
                let Ok((n, src)) = sock.recv_from(&mut buf) else { continue };
                if second.elapsed() >= Duration::from_secs(1) {
                    (second, seen) = (Instant::now(), 0);
                }
                // Поток запросов (с чужими подписями — на каждую проверку нужен ключ пары) не разбираем.
                seen += 1;
                if seen > MAX_QUERIES {
                    continue;
                }
                let Some(q) = parse_datagram(Q_MAGIC, &buf[..n]) else { continue };
                if let Some(a) = answer(&host, tcp_port, &q) {
                    let _ = sock.send_to(&datagram(A_MAGIC, &a), src);
                }
            }
        });
        Ok(Responder { port, stop, thread: Some(thread) })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Ищет в сети устройства с показанным кодом и связанные устройства (`peers` — номера и ключи пары).
/// `targets` — куда слать запрос: широковещательные адреса сетей и порт поиска.
pub fn discover(targets: &[SocketAddr], me: &str, peers: &[(String, [u8; 32])], wait: Duration) -> Result<Vec<Found>> {
    let sock = UdpSocket::bind(("0.0.0.0", 0))?;
    sock.set_broadcast(true)?;
    let mut nonces: BTreeMap<String, ([u8; 16], [u8; 32])> = BTreeMap::new();
    let mut packets = vec![datagram(Q_MAGIC, &json!({ "t": "pair", "from": me }))];
    for (id, key) in peers {
        let n: [u8; 16] = random()?;
        let m = mac(key, &[b"query", me.as_bytes(), id.as_bytes(), &n]);
        packets.push(datagram(Q_MAGIC, &json!({ "t": "peer", "from": me, "to": id, "n": b64(&n), "m": b64(&m) })));
        nonces.insert(id.clone(), (n, *key));
    }
    for t in targets {
        for p in &packets {
            // Сеть без широковещания или адрес недоступен — пробуем остальные.
            let _ = sock.send_to(p, t);
        }
    }
    let until = Instant::now() + wait;
    let mut found: BTreeMap<String, Found> = BTreeMap::new();
    let mut buf = [0u8; 1500];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        sock.set_read_timeout(Some(left.max(Duration::from_millis(1))))?;
        let Ok((n, src)) = sock.recv_from(&mut buf) else { continue };
        let Some(a) = parse_datagram(A_MAGIC, &buf[..n]) else { continue };
        let (Some(d), Some(p)) = (a["d"].as_str().filter(|d| valid_id(d) && *d != me), a["p"].as_u64().and_then(|p| u16::try_from(p).ok())) else { continue };
        let addr = SocketAddr::new(answer_ip(src.ip()), p);
        let app = a["app"].as_str().unwrap_or("").chars().take(16).collect::<String>();
        match a["t"].as_str() {
            Some("pair") => {
                let e = found.entry(d.to_string()).or_insert_with(|| Found { device: d.into(), name: String::new(), addr, pairing: true, peer: false, app: app.clone() });
                e.pairing = true;
                e.name = clean_name(a["name"].as_str().unwrap_or(""));
            }
            Some("peer") => {
                let Some((nq, key)) = nonces.get(d) else { continue };
                let Ok(m) = field::<32>(&a, "m") else { continue };
                if !same(&m, &mac(key, &[b"answer", d.as_bytes(), me.as_bytes(), nq, p.to_string().as_bytes()])) {
                    continue;
                }
                let e = found.entry(d.to_string()).or_insert_with(|| Found { device: d.into(), name: String::new(), addr, pairing: false, peer: true, app: app.clone() });
                e.peer = true;
                e.addr = addr;
            }
            _ => {}
        }
    }
    Ok(found.into_values().collect())
}

/// Адрес ответившего: IPv4 из IPv6-обёртки — как есть.
fn answer_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Title, KIND};
    use crate::Date;

    type Log = Arc<Mutex<Vec<(String, std::result::Result<SyncReport, String>)>>>;

    struct Side {
        store: Arc<Mutex<Store>>,
        host: Host,
        keys: Arc<Mutex<BTreeMap<String, [u8; 32]>>>,
        log: Log,
    }

    fn side(dir: &std::path::Path, device: &str, name: &str) -> Side {
        let store = Arc::new(Mutex::new(Store::open(&dir.join(format!("{device}.db")), &[5u8; 32], device).unwrap()));
        let keys: Arc<Mutex<BTreeMap<String, [u8; 32]>>> = Arc::default();
        let log: Log = Arc::default();
        let (k1, k2, l) = (keys.clone(), keys.clone(), log.clone());
        let host = Host {
            store: store.clone(),
            device: device.into(),
            name: name.into(),
            code: Arc::default(),
            peer_key: Arc::new(move |id| lock(&k1).get(id).copied()),
            paired: Arc::new(move |p, k| {
                lock(&k2).insert(p.device.clone(), *k);
                Ok(())
            }),
            synced: Arc::new(move |id, r| lock(&l).push((id.to_string(), r.clone()))),
        };
        Side { store, host, keys, log }
    }

    fn title(id: &str, name: &str) -> Title {
        let mut t = Title::new(Kind::Book, name, Date::parse("2026-11-14").unwrap());
        t.id = id.into();
        t
    }

    fn put(s: &Side, id: &str, name: &str) {
        lock(&s.store).put(KIND, id, &title(id, name)).unwrap();
    }

    fn count(s: &Side) -> usize {
        lock(&s.store).list::<Title>(KIND).unwrap().len()
    }

    fn local(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    /// Ждём, пока сервер в своём потоке допишет журнал.
    fn wait_log(s: &Side, n: usize) {
        let t0 = Instant::now();
        while lock(&s.log).len() < n && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn jpeg(seed: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend((0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)));
        v
    }

    #[test]
    fn pair_by_code_then_sync_by_pair_key() {
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let phone = side(dir.path(), "phone", "Телефон");
        put(&laptop, "a", "Пикник на обочине");
        put(&phone, "b", "Лавр");
        let server = Server::start(laptop.host.clone(), 0).unwrap();
        *lock(&laptop.host.code) = Some(CodeState::new("6WVGHEWK".into()));
        let pk = phone.keys.clone();
        let (peer, r) = pair_join(local(server.port), "6wvg-hewk", &phone.store, "phone", "Телефон", &|p, k| {
            lock(&pk).insert(p.device.clone(), *k);
            Ok(())
        })
        .unwrap();
        assert_eq!((peer.device.as_str(), peer.name.as_str(), peer.app.as_str()), ("laptop", "Ноутбук", APP_VERSION));
        assert_eq!((r.sent, r.received), (1, 1), "первый обмен — сразу при связи");
        wait_log(&laptop, 1);
        let kl = lock(&laptop.keys).get("phone").copied().expect("ноутбук запомнил телефон");
        let kp = lock(&phone.keys).get("laptop").copied().expect("телефон запомнил ноутбук");
        assert_eq!(kl, kp, "ключ пары у обоих один");
        assert!(lock(&laptop.host.code).is_none(), "код сработал один раз");
        assert_eq!(count(&laptop), 2);
        // Код больше не действует.
        let again = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|_, _| Ok(())).unwrap_err();
        assert!(matches!(again, Error::Lan(LanFail::NoCode, _)), "{again}");

        // Обмен по ключу пары — в обе стороны, повторный ничего не передаёт.
        put(&phone, "c", "Брат");
        let r = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &kp, &phone.store, "Телефон").unwrap();
        assert_eq!((r.peer.as_str(), r.sent, r.received), ("laptop", 1, 0));
        let r = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &kp, &phone.store, "Телефон").unwrap();
        assert_eq!((r.sent, r.received), (0, 0));
        // Чужой ключ: ни одной записи, понятная ошибка.
        put(&phone, "d", "Щегол");
        let bad = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &[7u8; 32], &phone.store, "Телефон").unwrap_err();
        assert!(matches!(bad, Error::Lan(LanFail::NotPeer, _) | Error::Io(_)), "{bad}");
        assert_eq!(count(&laptop), 3, "с чужим ключом ничего не пришло");
        // Не связанное устройство и не тот адресат.
        let unknown = sync_peer(local(server.port), "stranger", "laptop", "Ноутбук", &kp, &phone.store, "Чужой").unwrap_err();
        assert!(matches!(unknown, Error::Lan(LanFail::Unknown, ref m) if m.contains("«Ноутбук» не узнало")), "{unknown}");
        let other = sync_peer(local(server.port), "phone", "desktop", "Компьютер", &kp, &phone.store, "Телефон").unwrap_err();
        assert!(matches!(other, Error::Lan(LanFail::NotPeer, _)), "{other}");
        server.stop();
    }

    /// Обложки едут вместе с записями: и с телефона, и с компьютера, в несколько пакетов.
    #[test]
    fn covers_travel_with_titles() {
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let phone = side(dir.path(), "phone", "Телефон");
        let photo = jpeg(1, 300_000);
        let big: Vec<Vec<u8>> = (0..3).map(|i| jpeg(10 + i, 7 * 1024 * 1024)).collect();
        {
            let mut p = lock(&phone.store);
            for (i, data) in std::iter::once(&photo).chain(big.iter()).enumerate() {
                let mut t = title(&format!("p{i}"), "С телефона");
                t.cover = Some(p.put_blob("image/jpeg", data).unwrap());
                p.put(KIND, &t.id, &t).unwrap();
            }
        }
        let small = jpeg(50, 20_000);
        {
            let mut l = lock(&laptop.store);
            let mut t = title("l1", "С ноутбука");
            t.cover = Some(l.put_blob("image/jpeg", &small).unwrap());
            l.put(KIND, &t.id, &t).unwrap();
        }
        let server = Server::start(laptop.host.clone(), 0).unwrap();
        *lock(&laptop.host.code) = Some(CodeState::new("6WVGHEWK".into()));
        let (_, r) = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|_, _| Ok(())).unwrap();
        assert_eq!((r.files_sent, r.files_received), (4, 1), "{r:?}");
        assert!(lock(&phone.store).missing_blobs().unwrap().is_empty());
        assert!(lock(&laptop.store).missing_blobs().unwrap().is_empty());
        let id = crate::db::blob_id(&photo);
        assert_eq!(lock(&laptop.store).blob(&id).unwrap().unwrap(), ("image/jpeg".to_string(), photo));
        server.stop();
    }

    /// Файлы книг: каждая сторона забирает недостающие, если обмен файлами включён у обеих.
    #[test]
    fn book_files_travel_when_both_sides_agree() {
        use crate::files::{self, BookFile, FILE, SYNC_KEY};
        use crate::reader::BookFormat;
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let phone = side(dir.path(), "phone", "Телефон");
        for (s, name) in [(&laptop, "laptop"), (&phone, "phone")] {
            lock(&s.store).set_books_dir(dir.path().join(format!("{name}-books")));
        }
        // На ноутбуке — большой PDF (3 куска), на телефоне — маленький FB2.
        let pdf: Vec<u8> = b"%PDF-1.7\n".iter().copied().chain((0..(files::PART * 2 + 99)).map(|i| (i % 253) as u8)).collect();
        let fb2 = "<FictionBook>Лавр</FictionBook>".as_bytes().to_vec();
        let add = |s: &Side, tid: &str, data: &[u8], format: BookFormat, ext: &str| {
            let mut st = lock(&s.store);
            st.put(KIND, tid, &title(tid, "Книга")).unwrap();
            let id = crate::db::blob_id(data);
            let d = st.books_dir().unwrap().to_path_buf();
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{id}.{ext}")), data).unwrap();
            st.put(FILE, &id, &BookFile { id: id.clone(), title: tid.into(), name: format!("book.{ext}"), format, size: data.len() as u64, added: 1 }).unwrap();
            id
        };
        let pdf_id = add(&laptop, "t-pdf", &pdf, BookFormat::Pdf, "pdf");
        let fb2_id = add(&phone, "t-fb2", &fb2, BookFormat::Fb2, "fb2");
        let server = Server::start(laptop.host.clone(), 0).unwrap();
        *lock(&laptop.host.code) = Some(CodeState::new("6WVGHEWK".into()));
        let pk = phone.keys.clone();
        // Обмен файлами выключен: записи пришли, файлы — нет.
        let (_, r) = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|p, k| {
            lock(&pk).insert(p.device.clone(), *k);
            Ok(())
        })
        .unwrap();
        assert_eq!((r.books_sent, r.books_received), (0, 0));
        assert_eq!(files::wanted(&lock(&phone.store)).unwrap().len(), 1, "телефон знает о PDF");
        let kp = lock(&phone.keys).get("laptop").copied().unwrap();
        // Включили только на телефоне: ноутбук не отдаёт и не берёт.
        lock(&phone.store).set_meta(SYNC_KEY, "1").unwrap();
        let r = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &kp, &phone.store, "Телефон").unwrap();
        assert_eq!((r.books_sent, r.books_received), (0, 0));
        // Включили на обоих: PDF — на телефон, FB2 — на ноутбук.
        lock(&laptop.store).set_meta(SYNC_KEY, "1").unwrap();
        let r = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &kp, &phone.store, "Телефон").unwrap();
        assert_eq!((r.books_sent, r.books_received), (1, 1), "{r:?}");
        let got = files::path_in(lock(&phone.store).books_dir().unwrap(), &pdf_id).unwrap();
        assert_eq!(std::fs::read(got).unwrap(), pdf);
        assert!(files::path_in(lock(&laptop.store).books_dir().unwrap(), &fb2_id).is_some());
        assert_eq!(files::local(&lock(&laptop.store), &fb2_id).unwrap().title, "t-fb2");
        // Больше передавать нечего.
        let r = sync_peer(local(server.port), "phone", "laptop", "Ноутбук", &kp, &phone.store, "Телефон").unwrap();
        assert_eq!((r.books_sent, r.books_received), (0, 0));
        server.stop();
        assert!(crate::sync::speaks_files("0.8.0") && crate::sync::speaks_files("1.0") && !crate::sync::speaks_files("0.7.3"));
    }

    #[test]
    fn five_wrong_codes_cancel_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let phone = side(dir.path(), "phone", "Телефон");
        let server = Server::start(laptop.host.clone(), 0).unwrap();
        *lock(&laptop.host.code) = Some(CodeState::new("6WVGHEWK".into()));
        for left in (0..CODE_TRIES).rev() {
            let e = pair_join(local(server.port), "6WVG-HEWM", &phone.store, "phone", "Телефон", &|_, _| panic!("не должно связаться")).unwrap_err();
            assert!(matches!(e, Error::Lan(LanFail::WrongCode(l), _) if l == left), "{e}");
        }
        let st = lock(&laptop.host.code).clone().unwrap();
        assert_eq!((st.left, st.wrong), (0, 5));
        // И верный код больше не проходит.
        let e = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|_, _| panic!()).unwrap_err();
        assert!(matches!(e, Error::Lan(LanFail::Cancelled, ref m) if m.contains("пять неверных")), "{e}");
        assert!(lock(&laptop.keys).is_empty() && count(&laptop) == 0);
        // Истёкший код — как не показанный.
        *lock(&laptop.host.code) = Some(CodeState { until: Instant::now() - Duration::from_secs(1), ..CodeState::new("6WVGHEWK".into()) });
        let e = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|_, _| panic!()).unwrap_err();
        assert!(matches!(e, Error::Lan(LanFail::NoCode, _)), "{e}");
        server.stop();
    }

    /// Подставное устройство, которое не знает кода: связь не проходит, и оно ничего не получает.
    #[test]
    fn fake_host_without_code_gets_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fake = side(dir.path(), "fake", "Ноутбук");
        let phone = side(dir.path(), "phone", "Телефон");
        put(&phone, "a", "Секретная книга");
        let server = Server::start(fake.host.clone(), 0).unwrap();
        // У подставного свой код — человек вводит код с настоящего ноутбука.
        *lock(&fake.host.code) = Some(CodeState::new("ABCDEFGH".into()));
        let e = pair_join(local(server.port), "6WVGHEWK", &phone.store, "phone", "Телефон", &|_, _| panic!()).unwrap_err();
        assert!(matches!(e, Error::Lan(LanFail::WrongCode(4), _)), "{e}");
        assert!(count(&fake) == 0 && lock(&phone.keys).is_empty());
        server.stop();
    }

    #[test]
    fn packets_out_of_order_or_replayed_do_not_decrypt() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let a = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (b, _) = l.accept().unwrap();
        let mut tx = SessionWire::new(a, [1u8; 32], [2u8; 32]);
        tx.send(&Msg::Done { applied: 1 }).unwrap();
        tx.send(&Msg::Done { applied: 2 }).unwrap();
        let mut raw = b.try_clone().unwrap();
        let mut frames = Vec::new();
        for _ in 0..2 {
            let mut len = [0u8; 4];
            raw.read_exact(&mut len).unwrap();
            let mut f = vec![0u8; u32::from_be_bytes(len) as usize];
            raw.read_exact(&mut f).unwrap();
            frames.push(f);
        }
        // Второй пакет первым — не расшифруется.
        let l2 = TcpListener::bind("127.0.0.1:0").unwrap();
        let p2 = l2.local_addr().unwrap().port();
        let mut c = TcpStream::connect(("127.0.0.1", p2)).unwrap();
        let (d, _) = l2.accept().unwrap();
        for f in [&frames[1], &frames[0]] {
            c.write_all(&(f.len() as u32).to_be_bytes()).unwrap();
            c.write_all(f).unwrap();
        }
        let mut rx = SessionWire::new(d, [2u8; 32], [1u8; 32]);
        assert!(matches!(rx.recv(), Err(Error::Decrypt)), "пакет не на своём месте не расшифровывается");
        // Тот же первый пакет дважды: второй раз — уже не тот номер.
        let l3 = TcpListener::bind("127.0.0.1:0").unwrap();
        let p3 = l3.local_addr().unwrap().port();
        let mut e = TcpStream::connect(("127.0.0.1", p3)).unwrap();
        let (f, _) = l3.accept().unwrap();
        for _ in 0..2 {
            e.write_all(&(frames[0].len() as u32).to_be_bytes()).unwrap();
            e.write_all(&frames[0]).unwrap();
        }
        let mut rx = SessionWire::new(f, [2u8; 32], [1u8; 32]);
        assert!(matches!(rx.recv(), Ok(Msg::Done { applied: 1 })));
        assert!(matches!(rx.recv(), Err(Error::Decrypt)), "повтор пакета не проходит");
    }

    /// Объявленный огромный пакет без данных не занимает память: читается только то, что пришло.
    #[test]
    fn declared_big_frame_without_data_fails_fast() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut a = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (b, _) = l.accept().unwrap();
        a.write_all(&((MAX_FRAME as u32 - 1).to_be_bytes())).unwrap();
        a.write_all(&[1u8; 100]).unwrap();
        drop(a);
        let mut rx = SessionWire::new(b, [2u8; 32], [1u8; 32]);
        assert!(matches!(rx.recv(), Err(Error::Io(_))), "обрыв — ошибка, а не 64 МБ памяти");
    }

    #[test]
    fn server_drops_junk_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let server = Server::start(laptop.host.clone(), 0).unwrap();
        // Огромный открытый запрос — закрыто сразу, без чтения.
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        s.write_all(b"PLKLP\x01\xff\xff").unwrap();
        let t0 = Instant::now();
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 8];
        assert!(matches!(s.read(&mut buf), Ok(0) | Err(_)));
        assert!(t0.elapsed() < Duration::from_secs(2));
        // Запрос «Кармана» — тоже отказ: у приложений свои метки.
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        s.write_all(b"KRMLP\x01\x00\x02{}").unwrap();
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        assert!(matches!(s.read(&mut buf), Ok(0) | Err(_)));
        assert!(lock(&laptop.keys).is_empty() && lock(&laptop.log).is_empty());
        server.stop();
    }

    #[test]
    fn discovery_answers_only_with_code_or_to_peers() {
        let dir = tempfile::tempdir().unwrap();
        let laptop = side(dir.path(), "laptop", "Ноутбук");
        let key = [9u8; 32];
        lock(&laptop.keys).insert("phone".into(), key);
        let resp = Responder::start(laptop.host.clone(), 0, TCP_PORT).unwrap();
        let to = [local(resp.port)];
        let wait = Duration::from_millis(400);
        // Чужому без кода — тишина.
        assert!(discover(&to, "stranger", &[], wait).unwrap().is_empty(), "чужому в сети не отвечаем");
        // Связанному — ответ с адресом, подписанный ключом пары.
        let f = discover(&to, "phone", &[("laptop".into(), key)], wait).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].device.as_str(), f[0].peer, f[0].pairing, f[0].addr), ("laptop", true, false, local(TCP_PORT)));
        assert_eq!(f[0].name, "", "имя связанным не нужно и не отдаётся");
        // Подпись чужим ключом — тишина.
        assert!(discover(&to, "phone", &[("laptop".into(), [8u8; 32])], wait).unwrap().is_empty());
        // Показан код — отвечаем всем: имя и порт.
        *lock(&laptop.host.code) = Some(CodeState::new("6WVGHEWK".into()));
        let f = discover(&to, "stranger", &[], wait).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].name.as_str(), f[0].pairing, f[0].peer), ("Ноутбук", true, false));
        // Код отменён — снова тишина.
        lock(&laptop.host.code).as_mut().unwrap().left = 0;
        assert!(discover(&to, "stranger", &[], wait).unwrap().is_empty());
        resp.stop();
    }

    /// Подставной ответ на поиск без ключа пары не принимается.
    #[test]
    fn forged_answer_is_ignored() {
        let fake = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = fake.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            fake.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            for _ in 0..2 {
                let Ok((_, src)) = fake.recv_from(&mut buf) else { return };
                let a = json!({ "t": "peer", "d": "laptop", "p": TCP_PORT, "m": b64(&[0u8; 32]) });
                fake.send_to(&datagram(A_MAGIC, &a), src).unwrap();
            }
        });
        let f = discover(&[local(port)], "phone", &[("laptop".into(), [9u8; 32])], Duration::from_millis(500)).unwrap();
        assert!(f.is_empty(), "{f:?}");
        t.join().unwrap();
    }
}

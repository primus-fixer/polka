//! Связь по Wi-Fi — как в «Кармане» 0.15: связанные устройства, код связи, поиск в сети и обмен в фоне.
//!
//! - Связанные устройства и ключи пары — только на этом устройстве: в зашифрованной базе, ключи ещё
//!   и под защитой системы (DPAPI, Android Keystore). В обмен, облако и резервные копии не попадают.
//! - Сервер обмена и ответ на поиск работают, только пока есть связанные устройства или показан код.
//! - Связанные устройства в одной сети находят друг друга и обмениваются сами: вскоре после изменений
//!   и раз в минуту, пока «Полка» открыта.

use crate::{err, AppState, Res};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use polka_core::db::Store;
use polka_core::lan::{self, CodeState, Found, Host, Peer};
use polka_core::pairing::{self, is_local_address, Invite};
use polka_core::sync::SyncReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// На устройстве (не синхронизируется): связанные устройства.
const META_PEERS: &str = "lan.peers";
/// Связанных устройств — не больше стольких.
const MAX_PEERS: usize = 16;
/// Сколько ждать ответов на поиск.
const SCAN_WAIT: Duration = Duration::from_millis(1200);
/// Устройство «в сети», если его видели за это время.
const ONLINE: Duration = Duration::from_secs(90);
/// Обмен с найденным устройством — раз в минуту и через 5 секунд после своих изменений
/// (в отладочной сборке — POLKA_LAN_EVERY_SECS и POLKA_LAN_CHANGE_SECS).
const EVERY: u64 = 60;
const AFTER_CHANGE: u64 = 5;

/// Связанное устройство, как оно хранится.
#[derive(Clone, Serialize, Deserialize)]
struct PeerRec {
    id: String,
    name: String,
    /// Ключ пары под защитой системы, base64.
    key: String,
    /// Когда связали (unix).
    paired: i64,
    /// Последний удачный обмен (unix).
    #[serde(default)]
    last: Option<i64>,
    /// Где видели в последний раз: «192.168.1.34:47510».
    #[serde(default)]
    addr: Option<String>,
    #[serde(default)]
    app: String,
    /// Почему не получился последний обмен.
    #[serde(default)]
    error: Option<String>,
}

/// Состояние связи по Wi-Fi в приложении.
#[derive(Default)]
pub struct Lan {
    code: Arc<Mutex<Option<CodeState>>>,
    /// Сервер и ответчик на поиск.
    net: Mutex<Option<(lan::Server, Option<lan::Responder>)>>,
    /// Найдено при последнем поиске.
    found: Mutex<Vec<Found>>,
    /// Когда видели связанное устройство (найдено или был обмен).
    seen: Mutex<BTreeMap<String, Instant>>,
    /// С кем сейчас идёт обмен.
    busy: Mutex<Vec<String>>,
    /// Что сказать под кодом: «Связано с …», «Код отменён» — и когда (через две минуты не показываем).
    note: Mutex<Option<(String, Instant)>>,
}

static MANUAL: AtomicBool = AtomicBool::new(false);

struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

static WAKE: Wake = Wake { flag: Mutex::new(false), cv: Condvar::new() };

/// Поискать и обменяться при первой возможности.
pub fn wake() {
    MANUAL.store(true, Ordering::SeqCst);
    if let Ok(mut f) = WAKE.flag.lock() {
        *f = true;
    }
    WAKE.cv.notify_all();
}

fn sleep_or_wake(d: Duration) {
    let Ok(g) = WAKE.flag.lock() else {
        std::thread::sleep(d);
        return;
    };
    if let Ok((mut g, _)) = WAKE.cv.wait_timeout_while(g, d, |woken| !*woken) {
        *g = false;
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn load(st: &Store) -> Vec<PeerRec> {
    st.meta(META_PEERS).ok().flatten().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(st: &Store, peers: &[PeerRec]) -> Res<()> {
    st.set_meta(META_PEERS, &serde_json::to_string(peers).map_err(err)?).map_err(err)
}

fn key_of(rec: &PeerRec) -> Option<[u8; 32]> {
    let blob = B64.decode(&rec.key).ok()?;
    polka_secure::unprotect(&blob).ok()?.try_into().ok()
}

/// Обновить запись связанного устройства, если оно ещё связано.
fn update(app: &AppHandle, id: &str, f: impl FnOnce(&mut PeerRec)) {
    let state = app.state::<AppState>();
    let Ok(st) = state.store.lock() else { return };
    let mut peers = load(&st);
    if let Some(p) = peers.iter_mut().find(|p| p.id == id) {
        f(p);
        let _ = save(&st, &peers);
    }
}

/// Устройство в сети для сервера и ответчика: ключи пары и запоминание — из базы этого устройства.
fn host(app: &AppHandle) -> Host {
    let state = app.state::<AppState>();
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    Host {
        store: state.store.clone(),
        device: state.id.clone(),
        name: state.name.clone(),
        code: state.lan.code.clone(),
        peer_key: Arc::new(move |id| {
            let state = a1.state::<AppState>();
            let st = state.store.lock().ok()?;
            load(&st).iter().find(|p| p.id == id).and_then(key_of)
        }),
        paired: Arc::new(move |peer, key| {
            remember(&a2, peer, key)?;
            let state = a2.state::<AppState>();
            *lock(&state.lan.note) = Some((format!("Связано с «{}»", peer.name), Instant::now()));
            let _ = a2.emit("lan", LanEvent { received: 0, paired: Some(peer.name.clone()) });
            Ok(())
        }),
        synced: Arc::new(move |id, r| done(&a3, id, r)),
    }
}

/// Запомнить связанное устройство (или обновить ключ, если связали заново).
fn remember(app: &AppHandle, peer: &Peer, key: &[u8; 32]) -> Res<()> {
    let blob = polka_secure::protect(key).map_err(|e| format!("ключ связи не сохранить в хранилище системы: {e}"))?;
    let state = app.state::<AppState>();
    let st = state.store.lock().map_err(err)?;
    let mut peers = load(&st);
    peers.retain(|p| p.id != peer.device);
    if peers.len() >= MAX_PEERS {
        return Err(format!("Связанных устройств — не больше {MAX_PEERS}: отвяжите лишние"));
    }
    peers.push(PeerRec { id: peer.device.clone(), name: peer.name.clone(), key: B64.encode(blob), paired: now(), last: None, addr: None, app: peer.app.clone(), error: None });
    save(&st, &peers)?;
    drop(st);
    lock(&state.lan.seen).insert(peer.device.clone(), Instant::now());
    Ok(())
}

#[derive(Clone, Serialize)]
struct LanEvent {
    /// Сколько записей пришло — обновить сводку.
    received: usize,
    /// Связались по коду с этим устройством.
    paired: Option<String>,
}

/// Обмен закончился — запомнить, когда и с какой версией, и сказать экрану.
fn done(app: &AppHandle, id: &str, r: &Result<SyncReport, String>) {
    let state = app.state::<AppState>();
    lock(&state.lan.seen).insert(id.to_string(), Instant::now());
    match r {
        Ok(rep) => update(app, id, |p| {
            p.last = Some(now());
            p.error = None;
            if !rep.peer_app.is_empty() {
                p.app = rep.peer_app.clone();
            }
            if !rep.peer_name.is_empty() {
                p.name = rep.peer_name.chars().filter(|c| !c.is_control()).take(64).collect();
            }
        }),
        Err(e) => update(app, id, |p| p.error = Some(e.clone())),
    }
    let received = r.as_ref().map(|r| r.received + r.files_received + r.books_received).unwrap_or(0);
    let _ = app.emit("lan", LanEvent { received, paired: None });
}

/// Сервер и ответчик — пока есть связанные устройства или показан код; иначе порты закрыты.
fn ensure_net(app: &AppHandle) {
    let state = app.state::<AppState>();
    let want = lock(&state.lan.code).as_ref().is_some_and(CodeState::active) || state.store.lock().map(|st| !load(&st).is_empty()).unwrap_or(false);
    let mut net = lock(&state.lan.net);
    match (want, net.is_some()) {
        (true, false) => {
            let h = host(app);
            match lan::Server::start(h.clone(), lan::TCP_PORT) {
                Ok(server) => {
                    // Порт поиска занят (вторая «Полка» на компьютере) — нас просто не найдут, связь по адресу работает.
                    let responder = lan::Responder::start(h, discovery_port(), server.port).ok();
                    *net = Some((server, responder));
                }
                Err(e) => eprintln!("обмен по Wi-Fi: порт не открыть: {e}"),
            }
        }
        (false, true) => {
            if let Some((server, responder)) = net.take() {
                server.stop();
                if let Some(r) = responder {
                    r.stop();
                }
            }
        }
        _ => {}
    }
}

fn discovery_port() -> u16 {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var("POLKA_LAN_UDP").ok().and_then(|v| v.parse().ok()) {
        return p;
    }
    lan::DISCOVERY_PORT
}

fn tcp_port(state: &AppState) -> Option<u16> {
    lock(&state.lan.net).as_ref().map(|(s, _)| s.port)
}

/// Куда слать поиск: широковещательные адреса сетей устройства и общий 255.255.255.255.
fn targets() -> Vec<SocketAddr> {
    let port = lan::DISCOVERY_PORT;
    let mut out = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port)];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        if i.is_loopback() {
            continue;
        }
        if let if_addrs::IfAddr::V4(v4) = &i.addr {
            let b = v4.broadcast.unwrap_or_else(|| Ipv4Addr::from(u32::from(v4.ip) | !u32::from(v4.netmask)));
            let t = SocketAddr::new(IpAddr::V4(b), port);
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    // В отладочной сборке — ещё адреса для проверок (второе устройство на этом же компьютере).
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("POLKA_LAN_PROBE") {
        out.extend(v.split(',').filter_map(|a| a.trim().parse::<SocketAddr>().ok()));
    }
    out
}

/// Поиск в сети: устройства с показанным кодом и связанные.
fn scan(app: &AppHandle) -> Vec<Found> {
    let state = app.state::<AppState>();
    let keys: Vec<(String, [u8; 32])> = state.store.lock().map(|st| load(&st).iter().filter_map(|p| key_of(p).map(|k| (p.id.clone(), k))).collect()).unwrap_or_default();
    let found = lan::discover(&targets(), &state.id, &keys, SCAN_WAIT).unwrap_or_default();
    {
        let mut seen = lock(&state.lan.seen);
        for f in found.iter().filter(|f| f.peer) {
            seen.insert(f.device.clone(), Instant::now());
        }
    }
    for f in found.iter().filter(|f| f.peer) {
        let addr = f.addr.to_string();
        update(app, &f.device, |p| p.addr = Some(addr));
    }
    *lock(&state.lan.found) = found.clone();
    found
}

/// Обмен с одним связанным устройством.
fn sync_one(app: &AppHandle, rec: &PeerRec, addr: SocketAddr) -> Result<SyncReport, String> {
    let state = app.state::<AppState>();
    {
        let mut busy = lock(&state.lan.busy);
        if busy.contains(&rec.id) {
            return Err(format!("С «{}» уже идёт обмен", rec.name));
        }
        busy.push(rec.id.clone());
    }
    let key = key_of(rec).ok_or_else(|| "ключ связи не достать из хранилища системы: свяжите устройства заново".to_string());
    let r = key.and_then(|key| {
        lan::sync_peer(addr, &state.id, &rec.id, &rec.name, &key, &state.store, &state.name).map_err(|e| match e {
            polka_core::Error::Lan(_, m) | polka_core::Error::Peer(m) => m,
            e => format!("Не удалось обменяться с «{}»: {e}", rec.name),
        })
    });
    lock(&state.lan.busy).retain(|id| id != &rec.id);
    done(app, &rec.id, &r);
    r
}

/// Фон: открыть порты, когда нужно, находить связанные устройства и обмениваться.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let tick = secs("POLKA_LAN_TICK_SECS", 15);
        let every = secs("POLKA_LAN_EVERY_SECS", EVERY);
        let after_change = secs("POLKA_LAN_CHANGE_SECS", AFTER_CHANGE);
        let mut last_events: Option<u64> = None;
        let mut changed_at: Option<Instant> = None;
        let mut last_round: Option<Instant> = None;
        sleep_or_wake(secs("POLKA_LAN_DELAY_SECS", 3));
        loop {
            ensure_net(&app);
            let state = app.state::<AppState>();
            let (peers, events) = match state.store.lock() {
                Ok(st) => (load(&st), st.event_count().unwrap_or(0)),
                Err(_) => (Vec::new(), 0),
            };
            let manual = MANUAL.swap(false, Ordering::SeqCst);
            if last_events.is_some_and(|e| e != events) && changed_at.is_none() {
                changed_at = Some(Instant::now());
            }
            last_events = Some(events);
            let due = manual || last_round.is_none_or(|t| t.elapsed() >= every) || changed_at.is_some_and(|t| t.elapsed() >= after_change);
            if !peers.is_empty() && due {
                let found = scan(&app);
                for rec in &peers {
                    if let Some(f) = found.iter().find(|f| f.peer && f.device == rec.id) {
                        let _ = sync_one(&app, rec, f.addr);
                    }
                }
                last_round = Some(Instant::now());
                changed_at = None;
                last_events = state.store.lock().ok().and_then(|st| st.event_count().ok());
            }
            sleep_or_wake(tick);
        }
    });
}

fn secs(var: &str, default: u64) -> Duration {
    #[cfg(debug_assertions)]
    if let Some(s) = std::env::var(var).ok().and_then(|v| v.parse().ok()) {
        return Duration::from_secs(s);
    }
    let _ = var;
    Duration::from_secs(default)
}


// ——— Адреса, QR-код и версии ———

/// Адреса этого устройства в сети (IPv4): сначала домашние сети и точка доступа телефона.
pub(crate) fn local_ipv4() -> Vec<String> {
    let mut out: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if !v4.is_link_local() => Some(v4.to_string()),
            _ => None,
        })
        .collect();
    out.sort_by_key(|ip| if ip.starts_with("192.168.") { 0 } else if ip.starts_with("10.") { 1 } else { 2 });
    out.dedup();
    out
}

/// QR-код картинкой SVG.
fn qr_svg(text: &str) -> Res<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(err)?;
    Ok(code.render::<svg::Color>().min_dimensions(232, 232).dark_color(svg::Color("#15232D")).light_color(svg::Color("#FFFFFF")).build())
}

fn version_num(v: &str) -> (u32, u32, u32) {
    let mut it = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Что сказать, если версии на устройствах разные.
fn version_hint(peer_app: &str) -> String {
    let mine = crate::APP_VERSION;
    if peer_app.is_empty() {
        return String::new();
    }
    match version_num(peer_app).cmp(&version_num(mine)) {
        std::cmp::Ordering::Less => format!("На другом устройстве версия {peer_app} — обновите его до {mine}."),
        std::cmp::Ordering::Greater => format!("На другом устройстве версия {peer_app}, новее этой ({mine}) — обновите это устройство."),
        std::cmp::Ordering::Equal => String::new(),
    }
}

// ——— Команды ———

#[derive(Serialize)]
pub struct CodeView {
    /// «6WVG-HEWK»
    code: String,
    /// Сколько секунд ещё действует.
    secs: u64,
    /// Сколько попыток осталось.
    left: u8,
    wrong: u8,
    qr_svg: String,
    hosts: Vec<String>,
    port: u16,
}

#[derive(Serialize)]
pub struct PeerView {
    id: String,
    name: String,
    online: bool,
    syncing: bool,
    last: Option<i64>,
    app: String,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct NearbyView {
    device: String,
    name: String,
    addr: String,
}

#[derive(Serialize)]
pub struct LanView {
    /// Имя и адреса этого устройства.
    name: String,
    hosts: Vec<String>,
    code: Option<CodeView>,
    /// Под кодом: «Связано с …», «Код отменён…».
    note: Option<String>,
    peers: Vec<PeerView>,
    /// Устройства с показанным кодом, ещё не связанные.
    nearby: Vec<NearbyView>,
}

fn view(state: &AppState) -> Res<LanView> {
    let hosts = local_ipv4();
    let port = tcp_port(state).unwrap_or(lan::TCP_PORT);
    let code = {
        let mut c = lock(&state.lan.code);
        match c.as_ref() {
            Some(st) if st.active() => {
                let invite = Invite { device: state.id.clone(), name: state.name.clone(), hosts: hosts.clone(), port, code: st.code.clone() };
                Some(CodeView { code: pairing::show_code(&st.code), secs: st.until.saturating_duration_since(Instant::now()).as_secs(), left: st.left, wrong: st.wrong, qr_svg: qr_svg(&invite.to_uri())?, hosts: hosts.clone(), port })
            }
            Some(st) => {
                // Код истёк или отменён: убрать и сказать почему.
                let why = if st.left == 0 { "Код отменён: было пять неверных попыток. Покажите новый" } else { "Код больше не действует. Чтобы связать устройства, покажите новый" };
                *lock(&state.lan.note) = Some((why.into(), Instant::now()));
                c.take();
                None
            }
            None => None,
        }
    };
    let peers = {
        let st = state.store.lock().map_err(err)?;
        load(&st)
    };
    let seen = lock(&state.lan.seen).clone();
    let busy = lock(&state.lan.busy).clone();
    let found = lock(&state.lan.found).clone();
    Ok(LanView {
        name: state.name.clone(),
        hosts,
        code,
        note: lock(&state.lan.note).as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(120)).map(|(n, _)| n.clone()),
        peers: peers
            .iter()
            .map(|p| PeerView { id: p.id.clone(), name: p.name.clone(), online: seen.get(&p.id).is_some_and(|t| t.elapsed() < ONLINE), syncing: busy.contains(&p.id), last: p.last, app: p.app.clone(), error: p.error.clone() })
            .collect(),
        nearby: found.iter().filter(|f| f.pairing && !peers.iter().any(|p| p.id == f.device)).map(|f| NearbyView { device: f.device.clone(), name: f.name.clone(), addr: f.addr.to_string() }).collect(),
    })
}

/// Связь по Wi-Fi: код, связанные устройства, кто рядом.
#[tauri::command]
pub fn lan_status(state: State<AppState>) -> Res<LanView> {
    view(&state)
}

/// Поискать в сети сейчас (около секунды) — устройства с кодом и связанные.
#[tauri::command]
pub async fn lan_scan(app: AppHandle) -> Res<LanView> {
    let state = app.state::<AppState>();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || scan(&a)).await.map_err(err)?;
    view(&state)
}

/// Показать код связи: восемь знаков на пять минут, один раз.
#[tauri::command]
pub fn lan_code_start(app: AppHandle, state: State<AppState>) -> Res<LanView> {
    if local_ipv4().is_empty() {
        return Err("Нет подключения к сети: включите Wi-Fi или точку доступа".into());
    }
    *lock(&state.lan.code) = Some(CodeState::new(pairing::new_code().map_err(err)?));
    *lock(&state.lan.note) = None;
    ensure_net(&app);
    if tcp_port(&state).is_none() {
        lock(&state.lan.code).take();
        return Err("Не открыть порт для обмена: возможно, его заняла другая «Полка» на этом компьютере".into());
    }
    view(&state)
}

#[tauri::command]
pub fn lan_code_stop(app: AppHandle, state: State<AppState>) -> Res<LanView> {
    lock(&state.lan.code).take();
    *lock(&state.lan.note) = None;
    ensure_net(&app);
    view(&state)
}

#[derive(Serialize)]
pub struct PairResult {
    peer_name: String,
    sent: usize,
    received: usize,
    files_sent: usize,
    files_received: usize,
    /// Подсказка: другая версия на том устройстве.
    hint: String,
    /// Устройство запомнено.
    remembered: bool,
}

/// Ошибка ядра — предложением: без «неверный формат:», с большой буквы.
fn sentence(e: &polka_core::Error) -> String {
    let m = match e {
        polka_core::Error::Format(m) => m.clone(),
        e => e.to_string(),
    };
    let mut c = m.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => m,
    }
}

fn pair_error(e: polka_core::Error) -> String {
    match e {
        polka_core::Error::Lan(_, m) | polka_core::Error::Peer(m) => m,
        polka_core::Error::Io(e) => format!("Не удалось подключиться: {e}. Устройства должны быть в одной сети Wi-Fi"),
        e => format!("Не удалось связаться: {e}"),
    }
}

/// Связь по коду: с найденным устройством (`device`) или по адресу (`addr`: «192.168.1.34» или с портом).
#[tauri::command]
pub async fn lan_pair(app: AppHandle, device: Option<String>, addr: Option<String>, code: String) -> Res<PairResult> {
    let state = app.state::<AppState>();
    let code = pairing::normalize_code(&code).map_err(|e| sentence(&e))?;
    let target: SocketAddr = match (device, addr) {
        (Some(d), _) => lock(&state.lan.found).iter().find(|f| f.device == d && f.pairing).map(|f| f.addr).ok_or("Устройство пропало из сети: найдите его снова")?,
        (None, Some(a)) => parse_addr(&a)?,
        _ => return Err("Выберите устройство или введите его адрес".into()),
    };
    pair_at(app.clone(), vec![target], code).await
}

/// Адрес, введённый руками: только домашняя сеть.
fn parse_addr(a: &str) -> Res<SocketAddr> {
    let a = a.trim();
    let (host, port) = match a.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().map_err(|_| "Порт — число, например 192.168.1.34:47510".to_string())?),
        _ => (a, lan::TCP_PORT),
    };
    if !is_local_address(host) {
        return Err("Адрес — из домашней сети, как на экране того устройства: например 192.168.1.34".into());
    }
    Ok(SocketAddr::new(host.parse::<IpAddr>().map_err(err)?, port))
}

async fn pair_at(app: AppHandle, addrs: Vec<SocketAddr>, code: String) -> Res<PairResult> {
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let state = a.state::<AppState>();
        let mut last = None;
        for addr in addrs {
            match lan::pair_join(addr, &code, &state.store, &state.id, &state.name, &|peer, key| remember(&a, peer, key)) {
                Ok((peer, rep)) => {
                    done(&a, &peer.device, &Ok(rep.clone()));
                    let _ = a.emit("lan", LanEvent { received: rep.received + rep.files_received + rep.books_received, paired: Some(peer.name.clone()) });
                    return Ok((peer, rep));
                }
                // Нет связи по этому адресу — пробуем следующий; ответ устройства — окончательный.
                Err(polka_core::Error::Io(e)) => last = Some(polka_core::Error::Io(e)),
                Err(e) => return Err(pair_error(e)),
            }
        }
        Err(last.map(pair_error).unwrap_or_else(|| "Нет адреса для подключения".into()))
    })
    .await
    .map_err(err)??;
    wake();
    let (peer, rep) = r;
    Ok(PairResult { peer_name: peer.name, sent: rep.sent, received: rep.received, files_sent: rep.files_sent, files_received: rep.files_received, hint: version_hint(&rep.peer_app), remembered: true })
}

/// QR-код с другого устройства: адрес и код связи.
#[tauri::command]
pub async fn lan_pair_qr(app: AppHandle, uri: String) -> Res<PairResult> {
    let state = app.state::<AppState>();
    let inv = pairing::parse_qr(&uri).map_err(|e| sentence(&e))?;
    if inv.device == state.id {
        return Err("Это QR-код этого же устройства. Отсканируйте его другим устройством".into());
    }
    let addrs: Vec<SocketAddr> = inv.hosts.iter().filter_map(|h| h.parse::<IpAddr>().ok()).map(|ip| SocketAddr::new(ip, inv.port)).collect();
    pair_at(app.clone(), addrs, inv.code).await
}

/// Обменяться со связанным устройством сейчас.
#[tauri::command]
pub async fn lan_sync_now(app: AppHandle, id: String) -> Res<PairResult> {
    let a = app.clone();
    let rep = tauri::async_runtime::spawn_blocking(move || {
        let state = a.state::<AppState>();
        let rec = state.store.lock().map_err(err).map(|st| load(&st).into_iter().find(|p| p.id == id))?.ok_or("Устройство уже отвязано")?;
        // Где устройство сейчас: поиск в сети, а не нашли — прежний адрес.
        let found = scan(&a);
        let addr = found.iter().find(|f| f.peer && f.device == rec.id).map(|f| f.addr).or_else(|| rec.addr.as_deref().and_then(|x| x.parse().ok()));
        let addr = addr.ok_or_else(|| format!("«{}» не видно в сети: откройте «Полку» там и проверьте, что устройства в одной сети Wi-Fi", rec.name))?;
        sync_one(&a, &rec, addr).map(|r| (rec.name.clone(), r))
    })
    .await
    .map_err(err)??;
    let (name, r) = rep;
    Ok(PairResult { peer_name: name, sent: r.sent, received: r.received, files_sent: r.files_sent, files_received: r.files_received, hint: version_hint(&r.peer_app), remembered: true })
}

/// Отвязать устройство: обмен по Wi-Fi с ним прекращается, записи остаются на обоих.
#[tauri::command]
pub fn lan_unpair(app: AppHandle, state: State<AppState>, id: String) -> Res<LanView> {
    {
        let st = state.store.lock().map_err(err)?;
        let mut peers = load(&st);
        peers.retain(|p| p.id != id);
        save(&st, &peers)?;
    }
    lock(&state.lan.seen).remove(&id);
    ensure_net(&app);
    view(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_address_only_home_network() {
        assert_eq!(parse_addr(" 192.168.1.34 ").unwrap(), "192.168.1.34:47510".parse::<SocketAddr>().unwrap());
        assert_eq!(parse_addr("10.0.0.7:47600").unwrap(), "10.0.0.7:47600".parse::<SocketAddr>().unwrap());
        for bad in ["8.8.8.8", "evil.example.com", "93.184.216.34:47510", ""] {
            assert!(parse_addr(bad).unwrap_err().contains("из домашней сети"), "{bad}");
        }
        assert!(parse_addr("192.168.1.34:порт").unwrap_err().contains("Порт — число"));
    }

    #[test]
    fn search_goes_to_broadcast() {
        let t = targets();
        assert!(t.contains(&SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), lan::DISCOVERY_PORT)));
        assert!(t.iter().all(|a| a.port() == lan::DISCOVERY_PORT || cfg!(debug_assertions)));
    }

    #[test]
    fn core_errors_read_as_sentences() {
        let e = pairing::normalize_code("234").unwrap_err();
        assert_eq!(sentence(&e), "В коде восемь знаков, например 6WVG-HEWK");
        let e = pairing::parse_qr("https://example.com").unwrap_err();
        assert_eq!(sentence(&e), "Это не QR-код «Полки» для связи");
    }

    #[test]
    fn versions_and_qr() {
        assert_eq!(version_hint(crate::APP_VERSION), "");
        assert!(version_hint("0.0.1").contains("обновите его"));
        assert!(version_hint("99.0.0").contains("обновите это устройство"));
        assert!(qr_svg("polka://pair?v=1&d=a").unwrap().starts_with("<?xml"));
    }
}

//! Связь по Wi-Fi: поиск устройств в сети, связь по коду и обмен со связанными устройствами.
//!
//! С 0.11.7 всё устройство связи — общее, из kit (`kit_core::lan`): связь по коду (SPAKE2), ключ пары,
//! шифрованный канал сеанса и поиск в сети. Здесь — то, чем «Полка» отличается: свои порты, метки
//! и строки ключей ([`LAN`]), чтобы приложения в одной сети не мешали друг другу, и отчёт с файлами книг.
//! Менять настройки нельзя: устройства с прежней версией перестанут находить и узнавать друг друга.

use crate::db::Store;
use crate::error::Result;
use crate::sync::{SyncReport, PROFILE};
use kit_core::lan as kit;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use kit::{valid_id, CodeState, Found, Invite, OnPaired, Peer, CODE_TRIES, CODE_TTL};

/// Связь по Wi-Fi «Полки»: у «Кармана» порты 47500/47501 и метки KRM*, у «Полки» свои.
pub const LAN: kit::Lan = kit::Lan {
    names: crate::NAMES,
    profile: &PROFILE,
    tcp_port: 47510,
    discovery_port: 47511,
    magic: *b"PLKL",
    query_magic: *b"PLKQ",
    answer_magic: *b"PLKA",
    joiner: b"polka-joiner",
    host: b"polka-host",
    personal: b"polka-lan",
    not_ours: "это не «Полка»",
    qr_scheme: crate::pairing::SCHEME,
    qr_version: 1,
};

/// Что нужно связи по Wi-Fi в приложении от базы: местные настройки и число событий.
impl kit::Device for Store {
    fn local(&self, key: &str) -> Option<String> {
        self.meta(key).ok().flatten()
    }
    fn set_local(&self, key: &str, value: &str) -> kit_core::Result<()> {
        self.set_meta(key, value).map_err(|e| kit_core::Error::Store(e.to_string()))
    }
    fn event_count(&self) -> u64 {
        Store::event_count(self).unwrap_or(0)
    }
}

/// Порт обмена (TCP).
pub const TCP_PORT: u16 = LAN.tcp_port;
/// Порт поиска в сети (UDP).
pub const DISCOVERY_PORT: u16 = LAN.discovery_port;

type KeyFn = dyn Fn(&str) -> Option<[u8; 32]> + Send + Sync;
type PairedFn = dyn Fn(&Peer, &[u8; 32]) -> std::result::Result<(), String> + Send + Sync;
type SyncedFn = dyn Fn(&str, &std::result::Result<SyncReport, String>) + Send + Sync;

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
    /// То же устройство для общего модуля: отчёт — с файлами книг, ошибка — словами «Полки».
    fn kit(self) -> kit::Host<Store> {
        let synced = self.synced;
        kit::Host {
            lan: &LAN,
            store: self.store,
            device: self.device,
            name: self.name,
            code: self.code,
            peer_key: self.peer_key,
            paired: self.paired,
            synced: Arc::new(move |id, r| synced(id, &r.map(SyncReport::from).map_err(|e| crate::Error::from(e).to_string()))),
        }
    }
}

/// Связь по коду с устройством по адресу: ключ пары и первый обмен. `paired` запоминает устройство
/// сразу, как оба подтвердили код, — до обмена; ошибка в нём — обрыв без обмена.
pub fn pair_join(addr: SocketAddr, code: &str, store: &Mutex<Store>, device: &str, name: &str, paired: &OnPaired) -> Result<(Peer, SyncReport)> {
    let (peer, report) = kit::pair_join(&LAN, addr, code, store, device, name, paired)?;
    Ok((peer, report.into()))
}

/// Обмен со связанным устройством по ключу пары.
pub fn sync_peer(addr: SocketAddr, me: &str, peer: &str, peer_name: &str, key: &[u8; 32], store: &Mutex<Store>, name: &str) -> Result<SyncReport> {
    Ok(kit::sync_peer(&LAN, addr, me, peer, peer_name, key, store, name)?.into())
}

/// Приём подключений в фоне: связь по коду и обмен со связанными устройствами.
pub struct Server {
    pub port: u16,
    inner: kit::Server,
}

impl Server {
    /// Слушает `port`; занят — любой свободный.
    pub fn start(host: Host, port: u16) -> Result<Server> {
        let inner = kit::Server::start(host.kit(), port)?;
        Ok(Server { port: inner.port, inner })
    }

    pub fn stop(self) {
        self.inner.stop();
    }
}

/// Отвечает на поиск в сети, пока не остановят.
pub struct Responder {
    pub port: u16,
    inner: kit::Responder,
}

impl Responder {
    /// Слушает `port` (UDP); `tcp_port` — порт сервера для ответа.
    pub fn start(host: Host, port: u16, tcp_port: u16) -> Result<Responder> {
        let inner = kit::Responder::start(host.kit(), port, tcp_port)?;
        Ok(Responder { port: inner.port, inner })
    }

    pub fn stop(self) {
        self.inner.stop();
    }
}

/// Ищет в сети устройства с показанным кодом и связанные устройства (`peers` — номера и ключи пары).
/// `targets` — куда слать запрос: широковещательные адреса сетей и порт поиска.
pub fn discover(targets: &[SocketAddr], me: &str, peers: &[(String, [u8; 32])], wait: Duration) -> Result<Vec<Found>> {
    Ok(kit::discover(&LAN, targets, me, peers, wait)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Error, LanFail};
    use crate::sync::APP_VERSION;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpStream, UdpSocket};
    use std::sync::MutexGuard;
    use std::time::Instant;

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ключи связи выводятся как в 0.11: иначе устройства с прежней версией перестанут узнавать
    /// друг друга. Значения посчитаны независимо (BLAKE2b с личной строкой «polka-lan») и не меняются.
    #[test]
    fn keys_are_derived_as_before() {
        let hex = |b: [u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mac = |k: &[u8; 32], parts: &[&[u8]]| LAN.mac(k, parts);
        let k = [0x11u8; 32];
        let (nc, ns) = ([0x22u8; 32], [0x33u8; 32]);
        assert_eq!(hex(mac(&k, &[b"pair key"])), "0bdaaec39968032b1d3d4f267f1283d34bce5504340e6f3a28544770a4c27a43");
        assert_eq!(hex(mac(&k, &[b"joiner confirm"])), "70b0495f76d7f0ce7d789cdf83ee28fbf7d42d872a3329e409d1f121693e8be1");
        assert_eq!(hex(mac(&k, &[b"host confirm"])), "16549426e96bd48161ccb91e13103c2ce800be7b821bc73000a5cb8d304def46");
        assert_eq!(hex(mac(&k, &[b"sync c2s", &nc, &ns])), "051dad75907b35d30c34c1a660deabf656210bee5fe2ff0536b2b152f661c5ff");
        assert_eq!(hex(mac(&k, &[b"sync s2c", &nc, &ns])), "b065d8f30d2ab476e3b829a2775d6c07f28c5599459608a9deb201a39d6e0a8d");
        assert_eq!(hex(mac(&k, &[b"query", b"aaaa", b"bbbb", &nc])), "af63aedcbe1f33b2c858987f74ac46bdbc42ec950e870bc6c796ac5977c87733");
        assert_eq!(hex(mac(&k, &[b"answer", b"bbbb", b"aaaa", &nc, b"47510"])), "2d59f99b83a396f9cad53910b14e0e0832a68bd959e01b347402528a6121494e");
        assert_eq!((LAN.magic, LAN.query_magic, LAN.answer_magic), (*b"PLKL", *b"PLKQ", *b"PLKA"));
        assert_eq!((LAN.joiner, LAN.host), (&b"polka-joiner"[..], &b"polka-host"[..]));
        assert_eq!((TCP_PORT, DISCOVERY_PORT), (47510, 47511));
    }
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
                let a = json!({ "t": "peer", "d": "laptop", "p": TCP_PORT, "m": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=" });
                let mut d = b"PLKA\x01".to_vec();
                d.extend(serde_json::to_vec(&a).unwrap());
                fake.send_to(&d, src).unwrap();
            }
        });
        let f = discover(&[local(port)], "phone", &[("laptop".into(), [9u8; 32])], Duration::from_millis(500)).unwrap();
        assert!(f.is_empty(), "{f:?}");
        t.join().unwrap();
    }
}

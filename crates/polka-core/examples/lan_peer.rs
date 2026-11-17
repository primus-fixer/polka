//! Второе устройство для сквозных проверок связи по Wi-Fi: своя база в папке и команды.
//! Им проверяют, что разные версии «Полки» связываются и обмениваются в обе стороны.
//!
//! cargo run -p polka-core --example lan_peer -- <папка> <команда> …
//!   put <название> [обложка, КБ]    — записать книгу (с обложкой нужного размера)
//!   count                           — сколько записей, с какими названиями и каких обложек нет
//!   join <адрес:порт> <код>         — связаться по коду (запомнить ключ пары)
//!   sync <адрес:порт>               — обмен по ключу пары
//!   host <tcp> <udp> [код] [секунд] — ждать связи и обменов (с кодом — показать код)
//!   discover <адрес:порт,…>         — поиск в сети
//! Ответ — одна строка JSON.

use polka_core::db::Store;
use polka_core::lan::{self, CodeState, Host};
use polka_core::model::{Kind, Title, KIND};
use polka_core::Date;
use serde_json::json;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NAME: &str = "Планшет";

/// Номер устройства: LAN_PEER_ID или «peer-sim».
fn device() -> String {
    std::env::var("LAN_PEER_ID").unwrap_or_else(|_| "peer-sim".into())
}

fn store(dir: &Path) -> Arc<Mutex<Store>> {
    std::fs::create_dir_all(dir).unwrap();
    Arc::new(Mutex::new(Store::open(&dir.join("peer.db"), &[3u8; 32], &device()).unwrap()))
}

/// Ключ пары и номер связанного устройства — в файле рядом с базой.
fn saved(dir: &Path) -> Option<(String, [u8; 32])> {
    let t = std::fs::read_to_string(dir.join("pair.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    let key: Vec<u8> = v["key"].as_array()?.iter().filter_map(|x| x.as_u64().map(|b| b as u8)).collect();
    Some((v["peer"].as_str()?.to_string(), key.try_into().ok()?))
}

fn save(dir: &Path, peer: &str, key: &[u8; 32]) {
    std::fs::write(dir.join("pair.json"), json!({ "peer": peer, "key": key.to_vec() }).to_string()).unwrap();
}

fn out(v: serde_json::Value) {
    println!("{v}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]).to_path_buf();
    let st = store(&dir);
    match args[2].as_str() {
        "put" => {
            let id = polka_core::new_id().unwrap();
            let mut t = Title::new(Kind::Book, &args[3], Date::parse("2026-11-17").unwrap());
            t.id = id.clone();
            let mut s = st.lock().unwrap();
            if let Some(kb) = args.get(4).and_then(|k| k.parse::<usize>().ok()) {
                let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
                jpeg.extend((0..kb * 1024).map(|i| (i as u8).wrapping_mul(31).wrapping_add(id.as_bytes()[0])));
                t.cover = Some(s.put_blob("image/jpeg", &jpeg).unwrap());
            }
            s.put(KIND, &id, &t).unwrap();
            out(json!({ "ok": true, "id": id }));
        }
        "count" => {
            let s = st.lock().unwrap();
            let mut titles: Vec<String> = s.list::<Title>(KIND).unwrap().into_iter().map(|t| t.title).collect();
            titles.sort();
            out(json!({ "titles": titles, "missing_covers": s.missing_blobs().unwrap().len() }));
        }
        "join" => {
            let addr: SocketAddr = args[3].parse().unwrap();
            let d = dir.clone();
            match lan::pair_join(addr, &args[4], &st, &device(), NAME, &move |p, k| {
                save(&d, &p.device, k);
                Ok(())
            }) {
                Ok((peer, r)) => out(json!({ "ok": true, "peer": peer.device, "name": peer.name, "app": peer.app, "sent": r.sent, "received": r.received, "covers": [r.files_sent, r.files_received] })),
                Err(e) => out(json!({ "ok": false, "error": e.to_string() })),
            }
        }
        "sync" => {
            let addr: SocketAddr = args[3].parse().unwrap();
            let Some((peer, key)) = saved(&dir) else {
                return out(json!({ "ok": false, "error": "не связан" }));
            };
            match lan::sync_peer(addr, &device(), &peer, "Ноутбук", &key, &st, NAME) {
                Ok(r) => out(json!({ "ok": true, "sent": r.sent, "received": r.received, "covers": [r.files_sent, r.files_received], "app": r.peer_app })),
                Err(e) => out(json!({ "ok": false, "error": e.to_string() })),
            }
        }
        "host" => {
            let tcp: u16 = args[3].parse().unwrap();
            let udp: u16 = args[4].parse().unwrap();
            let code = args.get(5).filter(|c| !c.is_empty() && *c != "-").cloned();
            let secs: u64 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(60);
            let pair = Arc::new(Mutex::new(saved(&dir)));
            let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
            let (p1, p2, e1, e2, d) = (pair.clone(), pair.clone(), events.clone(), events.clone(), dir.clone());
            let host = Host {
                store: st.clone(),
                device: device(),
                name: NAME.into(),
                code: Arc::new(Mutex::new(code.map(CodeState::new))),
                peer_key: Arc::new(move |id| p1.lock().unwrap().as_ref().filter(|(p, _)| p == id).map(|(_, k)| *k)),
                paired: Arc::new(move |p, k| {
                    save(&d, &p.device, k);
                    *p2.lock().unwrap() = Some((p.device.clone(), *k));
                    e1.lock().unwrap().push(json!({ "paired": p.device, "name": p.name, "app": p.app }));
                    Ok(())
                }),
                synced: Arc::new(move |id, r| {
                    e2.lock().unwrap().push(match r {
                        Ok(r) => json!({ "synced": id, "sent": r.sent, "received": r.received, "covers": [r.files_sent, r.files_received] }),
                        Err(e) => json!({ "synced": id, "error": e }),
                    });
                }),
            };
            let server = lan::Server::start(host.clone(), tcp).unwrap();
            let responder = lan::Responder::start(host, udp, server.port).unwrap();
            println!("{}", json!({ "ready": true, "tcp": server.port, "udp": responder.port }));
            let until = Instant::now() + Duration::from_secs(secs);
            let mut shown = 0;
            while Instant::now() < until {
                std::thread::sleep(Duration::from_millis(100));
                let ev = events.lock().unwrap();
                for e in ev.iter().skip(shown) {
                    println!("{e}");
                }
                shown = ev.len();
            }
            server.stop();
            responder.stop();
        }
        "discover" => {
            let targets: Vec<SocketAddr> = args[3].split(',').map(|a| a.parse().unwrap()).collect();
            let peers: Vec<(String, [u8; 32])> = saved(&dir).into_iter().collect();
            let found = lan::discover(&targets, &device(), &peers, Duration::from_millis(1200)).unwrap();
            out(json!(found));
        }
        other => panic!("нет команды {other}"),
    }
}

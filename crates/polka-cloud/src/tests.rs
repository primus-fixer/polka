//! Проверки на имитации сервера WebDAV: вход по паролю, файлы, ошибки и обмен целиком.

use crate::dav::{self, Dav};
use crate::mock::{Mock, DAV_PASSWORD, DAV_USER};
use crate::now;
use polka_core::backup::Kdf;
use polka_core::cloud::{self, Cloud, Dir, Round, State};
use polka_core::db::Store;
use kit_core::cloud::CloudFail;
use kit_core::Error;
use polka_core::model::{Kind, Title, KIND};
use polka_core::Date;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

const FAST: Kdf = Kdf { m_kib: 8 * 1024, t: 1, p: 1 };

fn store(dir: &std::path::Path, name: &str, device: &str, demo: bool) -> Mutex<Store> {
    let mut s = Store::open(&dir.join(name), &[3u8; 32], device).unwrap();
    if demo {
        let mut photo = vec![9u8; 80_000];
        photo[..3].copy_from_slice(&[0xFF, 0xD8, 0xFF]);
        let cover = s.put_blob("image/jpeg", &photo).unwrap();
        for (i, name) in ["Пикник на обочине", "Лавр"].into_iter().enumerate() {
            let mut t = Title::new(Kind::Book, name, Date::parse("2026-11-14").unwrap());
            t.id = format!("t{i}");
            if i == 0 {
                t.cover = Some(cover.clone());
            }
            s.put(KIND, &t.id, &t).unwrap();
        }
    }
    Mutex::new(s)
}

fn round(c: &dyn Cloud, st: &Mutex<Store>, key: &cloud::CloudKey, state: &mut State) -> cloud::Report {
    let cancel = AtomicBool::new(false);
    cloud::sync(&Round { cloud: c, store: st, key, now: now(), cancel: &cancel }, state, &mut |_| {}).unwrap()
}

fn dav(m: &Mock) -> Dav {
    Dav::new(&dav::server_url(&m.dav_url(), true).unwrap(), DAV_USER, DAV_PASSWORD)
}

/// Обмен целиком через настоящие запросы: компьютер и телефон.
#[test]
fn phone_and_computer_exchange_through_webdav() {
    let m = Mock::start();
    let (c1, c2) = (dav(&m), dav(&m));
    let dir = tempfile::tempdir().unwrap();
    let pc = store(dir.path(), "pc.db", "aaaa", true);
    let phone = store(dir.path(), "phone.db", "bbbb", false);
    let (k1, created) = cloud::join(&c1, "пароль копий", FAST, now()).unwrap();
    assert!(created);
    let (k2, created) = cloud::join(&c2, "пароль копий", FAST, now()).unwrap();
    assert!(!created);
    let (mut st1, mut st2) = (State::default(), State::default());
    let r = round(&c1, &pc, &k1, &mut st1);
    assert_eq!((r.sent, r.files_sent), (2, 1));
    let r = round(&c2, &phone, &k2, &mut st2);
    assert_eq!((r.received, r.files_received, r.sent), (2, 1, 0));
    // Телефон добавил книгу — она приходит на компьютер.
    let mut t = Title::new(Kind::Book, "Щегол", Date::parse("2026-11-14").unwrap());
    t.id = "p1".into();
    phone.lock().unwrap().put(KIND, &t.id, &t).unwrap();
    assert_eq!(round(&c2, &phone, &k2, &mut st2).sent, 1);
    assert_eq!(round(&c1, &pc, &k1, &mut st1).received, 1);
    let (a, b) = (pc.lock().unwrap(), phone.lock().unwrap());
    assert_eq!(a.versions().unwrap(), b.versions().unwrap());
    let names = |s: &Store| {
        let mut v: Vec<String> = s.list::<Title>(KIND).unwrap().into_iter().map(|t| t.title).collect();
        v.sort();
        v
    };
    assert_eq!(names(&a), names(&b));
    assert!(b.missing_blobs().unwrap().is_empty());
    // В облаке — только зашифрованное: названий книг там не найти.
    for e in c1.list(Dir::Events).unwrap() {
        let data = c1.get(Dir::Events, &e.name, 1 << 26).unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&data).contains("Лавр"));
    }
    assert!(m.files().iter().all(|(n, _)| n.starts_with("dav:/Полка/")), "всё — в папке «Полка»");
}

#[test]
fn webdav_sign_in_with_password() {
    let m = Mock::start();
    let url = dav::server_url(&m.dav_url(), true).unwrap();
    let wrong = Dav::new(&url, DAV_USER, "not-the-password");
    assert!(matches!(wrong.connect(), Err(Error::Cloud(CloudFail::Auth, msg)) if msg.contains("не принял")));
    assert!(m.dav_dirs().is_empty(), "без входа ничего не создаётся");
    let d = dav(&m);
    assert!(!d.connect().unwrap(), "данных «Полки» ещё нет");
    assert_eq!(m.dav_dirs(), ["/Полка"]);
    assert!(!dav(&m).connect().unwrap(), "папка уже есть — не ошибка");
    d.put(Dir::Root, cloud::KEY_FILE, b"{}").unwrap();
    assert!(dav(&m).connect().unwrap(), "в облаке уже есть данные: пароль копий вводят, а не придумывают");
    let nowhere = Dav::new(&format!("{}/nowhere", m.base), DAV_USER, DAV_PASSWORD);
    assert!(matches!(nowhere.connect(), Err(Error::Cloud(CloudFail::Auth, msg)) if msg.contains("адрес")));
    m.dav_forbid(true);
    assert!(matches!(dav(&m).connect(), Err(Error::Cloud(CloudFail::Auth, msg)) if msg.contains("403")));
}

#[test]
fn webdav_files_in_every_folder() {
    let m = Mock::start();
    let s = dav(&m);
    assert!(s.list(Dir::Events).unwrap().is_empty(), "папки ещё нет — список пустой");
    assert_eq!(s.get(Dir::Root, cloud::KEY_FILE, 1024).unwrap(), None);
    assert!(!s.exists(Dir::Root, cloud::KEY_FILE).unwrap());
    s.put(Dir::Root, cloud::KEY_FILE, b"{\"k\":1}").unwrap();
    s.put(Dir::Events, "aaaa-1-3.pev", &[1, 2, 3]).unwrap();
    let big = vec![7u8; 6 << 20];
    s.put(Dir::Blobs, "b1.pbl", &big).unwrap();
    s.put(Dir::Events, "aaaa-1-3.pev", &[4, 5]).unwrap();
    assert!(s.exists(Dir::Root, cloud::KEY_FILE).unwrap());
    assert!(!s.exists(Dir::Root, "e").unwrap(), "папка — не файл");
    assert_eq!(s.get(Dir::Events, "aaaa-1-3.pev", 1024).unwrap(), Some(vec![4, 5]));
    assert_eq!(s.get(Dir::Blobs, "b1.pbl", 7 << 20).unwrap().map(|d| d.len()), Some(6 << 20));
    assert!(matches!(s.get(Dir::Blobs, "b1.pbl", 1024), Err(Error::Cloud(CloudFail::Format, _))), "больше предела — не скачиваем");
    let names: Vec<String> = s.list(Dir::Events).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["aaaa-1-3.pev"]);
    assert_eq!(s.list(Dir::Blobs).unwrap()[0].size, 6 << 20);
    assert_eq!(s.list(Dir::Root).unwrap().into_iter().map(|e| e.name).collect::<Vec<_>>(), [cloud::KEY_FILE], "папки e и b — не файлы");
    s.delete(Dir::Events, "aaaa-1-3.pev").unwrap();
    s.delete(Dir::Events, "aaaa-1-3.pev").unwrap();
    assert!(s.list(Dir::Events).unwrap().is_empty());
    let files: Vec<String> = m.files().into_iter().map(|(n, _)| n).collect();
    assert_eq!(files, ["dav:/Полка/b/b1.pbl", "dav:/Полка/polka.key"]);
    // Папку «Полка» удалили в облаке, пока «Полка» работала: она создаётся снова.
    m.dav_remove("/Полка");
    s.put(Dir::Events, "aaaa-4-4.pev", b"again").unwrap();
    assert_eq!(s.get(Dir::Events, "aaaa-4-4.pev", 1024).unwrap().as_deref(), Some(&b"again"[..]));
    // Данные удалили: пустые папки «Полки» убираются, чужие файлы остаются.
    s.put(Dir::Root, "заметки.txt", b"my notes").unwrap();
    cloud::wipe(&s, &AtomicBool::new(false)).unwrap();
    s.tidy().unwrap();
    assert_eq!(m.files().into_iter().map(|(n, _)| n).collect::<Vec<_>>(), ["dav:/Полка/заметки.txt"]);
    assert_eq!(m.dav_dirs(), ["/Полка"]);
    s.delete(Dir::Root, "заметки.txt").unwrap();
    s.tidy().unwrap();
    assert!(m.dav_dirs().is_empty());
}

#[test]
fn webdav_full_disk_new_password_and_no_connection() {
    let m = Mock::start();
    let s = dav(&m);
    s.put(Dir::Events, "aaaa-1-1.pev", b"x").unwrap();
    m.full(true);
    assert!(matches!(s.put(Dir::Blobs, "b.pbl", b"photo"), Err(Error::Cloud(CloudFail::Space, _))));
    m.full(false);
    m.dav_password(Some("another-app-password"));
    assert!(matches!(s.list(Dir::Events), Err(Error::Cloud(CloudFail::Auth, _))), "пароль приложения удалили — войти снова");
    m.dav_password(None);
    m.dav_forbid(true);
    assert!(matches!(s.exists(Dir::Root, cloud::KEY_FILE), Err(Error::Cloud(CloudFail::Auth, _))));
    assert!(m.requests() > 0);
    let url = dav::server_url(&m.dav_url(), true).unwrap();
    drop(m);
    assert!(matches!(Dav::new(&url, DAV_USER, DAV_PASSWORD).list(Dir::Events), Err(Error::Cloud(CloudFail::Offline, _))));
}

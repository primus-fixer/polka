//! WebDAV: Яндекс Диск по логину и паролю приложения или свой сервер (Nextcloud, NAS).
//!
//! «Полка» работает только в папке «Полка» в корне диска. Пароль приложения открывает весь Диск,
//! поэтому он хранится только на устройстве, под защитой системы. Файлы в папке зашифрованы ядром
//! (`polka_core::cloud`): без пароля копий их не прочитать. Устроено как в «Кармане».
//!
//! Нужно от сервера немногое: PROPFIND (список и проверка файла), GET, PUT, DELETE и MKCOL (папки).

use crate::http::{enc, fail, no_space, odd, server, Body, Http, Resp};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use polka_core::cloud::{Cloud, Dir, Entry, KEY_FILE};
use polka_core::error::{CloudFail, Error, Result};
use std::cell::RefCell;

/// Яндекс Диск по WebDAV.
pub const YANDEX: &str = "https://webdav.yandex.ru";
/// Папка «Полки» в корне диска.
pub const FOLDER: &str = "Полка";
/// Ответы без файлов маленькие.
const SMALL: usize = 1 << 20;
/// Список папки: на файл — около полукилобайта.
const LIST: usize = 32 << 20;
const PROPS: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/></d:prop></d:propfind>"#;

/// Имя сервера из адреса (с портом, если он есть): «nas.example.ru».
pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

/// Это Яндекс Диск: подсказки об ошибках — про пароль приложения.
pub fn is_yandex(url: &str) -> bool {
    matches!(host_of(url).to_ascii_lowercase().as_str(), "webdav.yandex.ru" | "webdav.yandex.com")
}

/// Проверяет и приводит адрес сервера: только https://, без «/» в конце; кириллица в пути кодируется.
/// `local` — можно http://127.0.0.1:… (имитация сервера в проверках).
pub fn server_url(raw: &str, local: bool) -> std::result::Result<String, String> {
    let u = raw.trim().trim_end_matches('/');
    let lower = u.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &u[8..]
    } else if local && lower.starts_with("http://127.0.0.1:") {
        &u[7..]
    } else {
        return Err("Адрес сервера начинается с https:// — без шифрования пароль ушёл бы по сети открытым".into());
    };
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, String::new()));
    let bad_host = host.is_empty() || host.contains('@') || !host.chars().all(|c| c.is_ascii_alphanumeric() || "-.:[]".contains(c));
    if bad_host || u.contains(['?', '#']) || u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Проверьте адрес сервера: например, https://cloud.example.ru/remote.php/dav/files/имя".into());
    }
    let mut out = format!("{}://{}", if lower.starts_with("https://") { "https" } else { "http" }, host.to_ascii_lowercase());
    for ch in path.chars() {
        if ch.is_ascii_graphic() && !"\"<>\\^`{|}".contains(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Ok(out)
}

/// Подключение к серверу WebDAV для одного обмена.
pub struct Dav {
    http: Http,
    server: String,
    /// Адрес папки «Полка».
    base: String,
    auth: String,
    yandex: bool,
    /// Папки, которые уже есть или созданы за этот обмен.
    dirs: RefCell<Vec<Dir>>,
}

impl Dav {
    /// `server` — адрес, проверенный [`server_url`].
    pub fn new(server: &str, user: &str, password: &str) -> Dav {
        let auth = format!("Basic {}", B64.encode(format!("{user}:{password}")));
        Dav { http: Http::new(), server: server.to_string(), base: format!("{server}/{}", enc(FOLDER)), auth, yandex: is_yandex(server), dirs: RefCell::default() }
    }

    fn folder_url(&self, dir: Dir) -> String {
        match dir {
            Dir::Root => format!("{}/", self.base),
            d => format!("{}/{}/", self.base, d.folder()),
        }
    }

    fn file_url(&self, dir: Dir, name: &str) -> String {
        match dir {
            Dir::Root => format!("{}/{}", self.base, enc(name)),
            d => format!("{}/{}/{}", self.base, d.folder(), enc(name)),
        }
    }

    fn send(&self, method: &str, url: &str, extra: &[(&str, &str)], body: Body, max: usize) -> Result<Resp> {
        let mut headers: Vec<(&str, &str)> = vec![("Authorization", &self.auth)];
        headers.extend_from_slice(extra);
        self.http.send(method, url, &headers, body, max)
    }

    /// Свойства папки (`depth` 1 — с содержимым) или файла (`depth` 0).
    fn propfind(&self, url: &str, depth: &str, max: usize) -> Result<Option<Vec<Item>>> {
        let r = self.send("PROPFIND", url, &[("Depth", depth)], Body::Xml(PROPS), max)?;
        match r.status {
            207 => multistatus(&String::from_utf8_lossy(&r.body)).map(Some).ok_or_else(odd),
            404 => Ok(None),
            _ => Err(self.trouble(&r)),
        }
    }

    fn denied(&self) -> Error {
        if self.yandex {
            fail(
                CloudFail::Auth,
                "Яндекс не принял логин или пароль приложения. Нужен пароль приложения для Диска (тип «Файлы WebDAV»), а не пароль от аккаунта. Новый пароль начинает действовать через 2–3 часа",
            )
        } else {
            fail(CloudFail::Auth, "Сервер не принял логин или пароль")
        }
    }

    /// Общие ответы с ошибкой.
    fn trouble(&self, r: &Resp) -> Error {
        match r.status {
            401 => self.denied(),
            403 if self.yandex => fail(CloudFail::Auth, "Яндекс Диск не пускает по WebDAV (ответ 403). Такой вход есть в тарифах Яндекс 360 — проверьте подписку и пароль приложения"),
            403 => fail(CloudFail::Auth, "Сервер не дал доступа к папке «Полка» (ответ 403) — проверьте права пользователя"),
            413 | 507 => no_space(),
            code => server(code),
        }
    }

    /// Проверяет вход и создаёт папку «Полка», если её нет. Возвращает, есть ли в ней данные «Полки».
    pub fn connect(&self) -> Result<bool> {
        if self.propfind(&format!("{}/", self.server), "0", SMALL)?.is_none() {
            return Err(fail(CloudFail::Auth, "По этому адресу нет папки WebDAV — проверьте адрес сервера"));
        }
        self.ensure(Dir::Root)?;
        self.exists(Dir::Root, KEY_FILE)
    }

    fn mkcol(&self, dir: Dir) -> Result<Resp> {
        self.send("MKCOL", &self.folder_url(dir), &[], Body::Empty, SMALL)
    }

    /// Создаёт папку, если её ещё нет.
    fn ensure(&self, dir: Dir) -> Result<()> {
        if self.dirs.borrow().contains(&dir) {
            return Ok(());
        }
        if dir != Dir::Root {
            self.ensure(Dir::Root)?;
        }
        let mut r = self.mkcol(dir)?;
        if r.status == 409 && dir != Dir::Root {
            // Папку «Полка» удалили, пока «Полка» работала: создаём снова.
            self.dirs.borrow_mut().retain(|d| *d != Dir::Root);
            self.ensure(Dir::Root)?;
            r = self.mkcol(dir)?;
        }
        // 405 — папка уже есть.
        if !r.ok() && r.status != 405 {
            return Err(self.trouble(&r));
        }
        self.dirs.borrow_mut().push(dir);
        Ok(())
    }

    /// После удаления данных из облака: убирает пустые папки «Полки». Чужие файлы не трогает.
    pub fn tidy(&self) -> Result<()> {
        for dir in [Dir::Events, Dir::Blobs, Dir::Root] {
            let Some(items) = self.propfind(&self.folder_url(dir), "1", LIST)? else { continue };
            // Сама папка — первый ответ; другого ничего нет — папка пустая.
            if items.len() <= 1 {
                let r = self.send("DELETE", &self.folder_url(dir), &[], Body::Empty, SMALL)?;
                if !r.ok() && r.status != 404 {
                    return Err(self.trouble(&r));
                }
            }
        }
        self.dirs.borrow_mut().clear();
        Ok(())
    }
}

impl Cloud for Dav {
    fn list(&self, dir: Dir) -> Result<Vec<Entry>> {
        let Some(items) = self.propfind(&self.folder_url(dir), "1", LIST)? else { return Ok(Vec::new()) };
        Ok(items.into_iter().filter(|i| !i.folder).filter_map(|i| Some(Entry { name: file_name(&i.href)?, size: i.size })).collect())
    }

    fn get(&self, dir: Dir, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        let r = self.send("GET", &self.file_url(dir, name), &[], Body::Empty, max)?;
        match r.status {
            200 => Ok(Some(r.body)),
            404 => Ok(None),
            _ => Err(self.trouble(&r)),
        }
    }

    fn put(&self, dir: Dir, name: &str, data: &[u8]) -> Result<()> {
        self.ensure(dir)?;
        let url = self.file_url(dir, name);
        let mut r = self.send("PUT", &url, &[], Body::Bytes(data), SMALL)?;
        if r.status == 409 {
            // Папку удалили, пока «Полка» работала: создаём снова.
            self.dirs.borrow_mut().clear();
            self.ensure(dir)?;
            r = self.send("PUT", &url, &[], Body::Bytes(data), SMALL)?;
        }
        if r.ok() {
            Ok(())
        } else {
            Err(self.trouble(&r))
        }
    }

    fn exists(&self, dir: Dir, name: &str) -> Result<bool> {
        Ok(self.propfind(&self.file_url(dir, name), "0", SMALL)?.is_some_and(|items| items.iter().any(|i| !i.folder)))
    }

    fn delete(&self, dir: Dir, name: &str) -> Result<()> {
        let r = self.send("DELETE", &self.file_url(dir, name), &[], Body::Empty, SMALL)?;
        if r.ok() || r.status == 404 {
            Ok(())
        } else {
            Err(self.trouble(&r))
        }
    }
}

/// Один ответ из списка PROPFIND.
#[derive(Debug, PartialEq)]
struct Item {
    href: String,
    size: u64,
    folder: bool,
}

/// Разбирает ответ PROPFIND (multistatus). Приставки пространств имён у серверов разные
/// (`d:`, `D:`, `lp1:` или пространство по умолчанию) — смотрим только на местные имена.
/// Не multistatus — `None`.
fn multistatus(xml: &str) -> Option<Vec<Item>> {
    let mut out = Vec::new();
    let mut seen = false;
    let mut cur: Option<Item> = None;
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map(|e| &after[e + 3..]).unwrap_or("");
            continue;
        }
        let gt = rest.find('>')?;
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let closing = tag.starts_with('/');
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();
        let empty = tag.ends_with('/');
        match (local.as_str(), closing) {
            ("multistatus", false) => seen = true,
            ("response", false) if !empty => cur = Some(Item { href: String::new(), size: 0, folder: false }),
            ("response", true) => out.extend(cur.take()),
            ("href", false) if !empty => {
                if let Some(i) = cur.as_mut().filter(|i| i.href.is_empty()) {
                    i.href = unescape(text(rest));
                }
            }
            ("getcontentlength", false) if !empty => {
                if let Some(i) = cur.as_mut() {
                    i.size = unescape(text(rest)).trim().parse().unwrap_or(0);
                }
            }
            ("collection", false) => {
                if let Some(i) = cur.as_mut() {
                    i.folder = true;
                }
            }
            _ => {}
        }
    }
    seen.then_some(out)
}

fn text(rest: &str) -> &str {
    &rest[..rest.find('<').unwrap_or(rest.len())]
}

/// Сущности XML: `&amp;`, `&lt;`, `&#1050;`, `&#x41A;`…
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Имя файла из ссылки ответа: последняя часть пути, раскодированная.
fn file_name(href: &str) -> Option<String> {
    let path = href.split(['?', '#']).next()?;
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    let b = last.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok().filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses() {
        assert_eq!(server_url(" https://webdav.yandex.ru/ ", false).unwrap(), "https://webdav.yandex.ru");
        assert_eq!(server_url("HTTPS://Cloud.Example.ru/remote.php/dav/files/Пётр", false).unwrap(), "https://cloud.example.ru/remote.php/dav/files/%D0%9F%D1%91%D1%82%D1%80");
        assert_eq!(server_url("https://nas.local:5006/home", false).unwrap(), "https://nas.local:5006/home");
        assert!(server_url("http://nas.local/home", false).unwrap_err().contains("https://"));
        assert!(server_url("http://127.0.0.1:18080/dav", false).is_err(), "без шифрования — только имитация в проверках");
        assert_eq!(server_url("http://127.0.0.1:18080/dav", true).unwrap(), "http://127.0.0.1:18080/dav");
        for bad in ["https://", "https://user:pass@host/dav", "https://host/dav?x=1", "https://ho st/dav", "webdav.yandex.ru", "ftp://host"] {
            assert!(server_url(bad, false).is_err(), "{bad}");
        }
        assert!(is_yandex("https://webdav.yandex.ru") && is_yandex("https://WEBDAV.yandex.com/x"));
        assert!(!is_yandex("https://webdav.yandex.ru.example.com") && !is_yandex("https://example.com/webdav.yandex.ru"));
        assert_eq!(host_of("https://nas.local:5006/home"), "nas.local:5006");
    }

    #[test]
    fn hints_for_yandex_disk() {
        let msg = |d: &Dav, status: u16| d.trouble(&Resp { status, body: Vec::new() }).to_string();
        let ya = Dav::new(YANDEX, "petrov", "abcdefghijklmnop");
        assert!(msg(&ya, 401).contains("пароль приложения") && msg(&ya, 401).contains("2–3 часа"));
        assert!(msg(&ya, 403).contains("Яндекс 360"));
        let own = Dav::new("https://nas.example.ru/dav", "petrov", "secret");
        assert!(msg(&own, 401).contains("не принял логин или пароль") && !msg(&own, 401).contains("Яндекс"));
        assert!(msg(&own, 507).contains("места"));
        assert_eq!(own.file_url(Dir::Events, "aaaa-1-3.pev"), "https://nas.example.ru/dav/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/e/aaaa-1-3.pev");
        assert_eq!(own.folder_url(Dir::Root), "https://nas.example.ru/dav/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/");
    }

    #[test]
    fn answers_of_different_servers() {
        // Яндекс Диск.
        let yandex = r#"<?xml version='1.0' encoding='UTF-8'?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/e/</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response><d:response><d:href>/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/e/aaaa-1-3.pev</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop><d:resourcetype/><d:getcontentlength>1234</d:getcontentlength></d:prop></d:propstat></d:response></d:multistatus>"#;
        let items = multistatus(yandex).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].folder && !items[1].folder);
        assert_eq!((file_name(&items[1].href).unwrap().as_str(), items[1].size), ("aaaa-1-3.pev", 1234));
        // Apache mod_dav: другие приставки, полные адреса и свойства, которых нет (404).
        let apache = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:ns0="DAV:">
<D:response xmlns:lp1="DAV:" xmlns:lp2="http://apache.org/dav/props/">
<D:href>https://nas.example.ru/dav/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/b/</D:href>
<D:propstat><D:prop><lp1:resourcetype><D:collection/></lp1:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
<D:propstat><D:prop><lp1:getcontentlength/></D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>
</D:response>
<!-- обложка <b>книги</b> -->
<D:response xmlns:lp1="DAV:">
<D:href>https://nas.example.ru/dav/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/b/0f0e.pbl</D:href>
<D:propstat><D:prop><lp1:resourcetype/><lp1:getcontentlength>80046</lp1:getcontentlength></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response>
</D:multistatus>"#;
        let items = multistatus(apache).unwrap();
        assert_eq!(items.iter().map(|i| (file_name(&i.href).unwrap(), i.size, i.folder)).collect::<Vec<_>>(), [("b".to_string(), 0, true), ("0f0e.pbl".to_string(), 80046, false)]);
        // Nextcloud: пространство имён по умолчанию и сущности в адресе.
        let next = r#"<?xml version="1.0"?><multistatus xmlns="DAV:" xmlns:oc="http://owncloud.org/ns"><response><href>/remote.php/dav/files/petrov/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/polka.key?x=1&amp;y=2</href><propstat><prop><resourcetype/><getcontentlength>512</getcontentlength><oc:size>512</oc:size></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        let items = multistatus(next).unwrap();
        assert_eq!((file_name(&items[0].href).unwrap().as_str(), items[0].size, items[0].folder), ("polka.key", 512, false));
        // Не multistatus: страница входа в сеть, ошибка прокси.
        assert_eq!(multistatus("<html><body>Войдите в Wi-Fi</body></html>"), None);
        assert_eq!(multistatus("not xml"), None);
        assert_eq!(unescape("a&amp;b &lt;c&gt; &#1050;&#x430; &bogus; &"), "a&b <c> Ка &bogus; &");
        assert_eq!(file_name("/dav/%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0/"), Some("Полка".into()));
        assert_eq!(file_name("/dav/50%25"), Some("50%".into()));
        assert_eq!(file_name("/dav/a%2"), Some("a%2".into()));
    }
}

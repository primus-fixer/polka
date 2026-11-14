//! Имитация сервера WebDAV (`/dav/…`) с входом по логину и паролю, как Яндекс Диск по паролю
//! приложения. Для проверок и сквозных тестов; в выпускную сборку не входит. Как в «Кармане».
//!
//! Управление (для сквозных тестов): `/mock/full?on=1` — на диске нет места, `GET /mock/files`;
//! `/mock/dav-password?value=…` — другой пароль (прежний пароль приложения удалили),
//! `/mock/dav-forbid?on=1` — сервер не пускает (403), `/mock/dav-remove?path=/Полка` — удалить папку.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Default)]
struct State {
    full: bool,
    /// «/Полка/e/имя» → содержимое; папки «/Полка», «/Полка/e».
    dav: BTreeMap<String, Vec<u8>>,
    dav_dirs: BTreeSet<String>,
    /// Пароль, если его сменили.
    dav_password: Option<String>,
    dav_forbid: bool,
    requests: usize,
}

/// Логин и пароль имитации WebDAV.
pub const DAV_USER: &str = "polka-test";
pub const DAV_PASSWORD: &str = "abcdefghijklmnop";

/// Сервер-имитация на 127.0.0.1.
pub struct Mock {
    pub base: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn lock(st: &Mutex<State>) -> MutexGuard<'_, State> {
    st.lock().unwrap_or_else(|e| e.into_inner())
}

impl Mock {
    pub fn start() -> Mock {
        Mock::start_on(0).expect("порт для имитации облака")
    }

    pub fn start_on(port: u16) -> std::io::Result<Mock> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        listener.set_nonblocking(true)?;
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !sp.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let st = st.clone();
                        std::thread::spawn(move || {
                            let _ = s.set_nonblocking(false);
                            let _ = serve(s, &st);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        });
        Ok(Mock { base, state, stop, thread: Some(thread) })
    }

    pub fn full(&self, yes: bool) {
        lock(&self.state).full = yes;
    }

    /// Все файлы: «dav:/Полка/e/имя», размер.
    pub fn files(&self) -> Vec<(String, usize)> {
        files_of(&lock(&self.state))
    }

    pub fn requests(&self) -> usize {
        lock(&self.state).requests
    }

    /// Адрес сервера WebDAV.
    pub fn dav_url(&self) -> String {
        format!("{}/dav", self.base)
    }

    /// Другой пароль: прежний пароль приложения удалили. `None` — снова прежний.
    pub fn dav_password(&self, p: Option<&str>) {
        lock(&self.state).dav_password = p.map(str::to_string);
    }

    /// Сервер не пускает (403).
    pub fn dav_forbid(&self, yes: bool) {
        lock(&self.state).dav_forbid = yes;
    }

    /// Папки на сервере.
    pub fn dav_dirs(&self) -> Vec<String> {
        lock(&self.state).dav_dirs.iter().cloned().collect()
    }

    /// Удаляет папку или файл, как будто это сделал человек в облаке.
    pub fn dav_remove(&self, path: &str) {
        dav_remove(&mut lock(&self.state), path);
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn files_of(st: &State) -> Vec<(String, usize)> {
    st.dav.iter().map(|(p, d)| (format!("dav:{p}"), d.len())).collect()
}

struct Req {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Req {
    fn q(&self, k: &str) -> &str {
        self.query.get(k).map(String::as_str).unwrap_or("")
    }

    fn header(&self, k: &str) -> &str {
        self.headers.get(k).map(String::as_str).unwrap_or("")
    }
}

struct Out {
    status: u16,
    body: Vec<u8>,
}

fn raw(status: u16, body: Vec<u8>) -> Out {
    Out { status, body }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_pairs(s: &str) -> BTreeMap<String, String> {
    s.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').map(|(k, v)| (decode(k), decode(v))).unwrap_or_else(|| (decode(p), String::new()))).collect()
}

fn read_req(stream: &mut TcpStream) -> std::io::Result<Option<Req>> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = BTreeMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    if headers.get("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut body = Vec::new();
    if headers.get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            let mut chunk = vec![0u8; n + 2];
            r.read_exact(&mut chunk)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else if let Some(n) = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body = vec![0u8; n];
        r.read_exact(&mut body)?;
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_pairs(q)),
        None => (target, BTreeMap::new()),
    };
    Ok(Some(Req { method, path, query, headers, body }))
}

fn serve(mut stream: TcpStream, st: &Mutex<State>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let Some(req) = read_req(&mut stream)? else { return Ok(()) };
    let out = {
        let mut st = lock(st);
        st.requests += 1;
        route(&req, &mut st)
    };
    let reason = match out.status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        207 => "Multi-Status",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        507 => "Insufficient Storage",
        _ => "Other",
    };
    let head = format!("HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/xml\r\n\r\n", out.status, out.body.len());
    stream.write_all(head.as_bytes())?;
    stream.write_all(&out.body)?;
    stream.flush()
}

fn route(req: &Req, st: &mut State) -> Out {
    let p = req.path.as_str();
    match p {
        "/mock/full" => {
            st.full = req.q("on") == "1";
            raw(200, Vec::new())
        }
        "/mock/dav-password" => {
            st.dav_password = Some(req.q("value").to_string()).filter(|v| !v.is_empty());
            raw(200, Vec::new())
        }
        "/mock/dav-forbid" => {
            st.dav_forbid = req.q("on") == "1";
            raw(200, Vec::new())
        }
        "/mock/dav-remove" => {
            dav_remove(st, req.q("path"));
            raw(200, Vec::new())
        }
        "/mock/files" => {
            let list: Vec<String> = files_of(st).into_iter().map(|(n, s)| format!("{{\"name\":\"{n}\",\"size\":{s}}}")).collect();
            raw(200, format!("[{}]", list.join(",")).into_bytes())
        }
        _ if p == "/dav" || p.starts_with("/dav/") => dav(req, st),
        _ => raw(404, b"Not Found".to_vec()),
    }
}

/// Путь из адреса: «%D0%9F…» раскодируется, «+» остаётся плюсом.
fn dav_path(raw: &str) -> String {
    let b = raw.as_bytes();
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
    let p = String::from_utf8_lossy(&out).trim_end_matches('/').to_string();
    if p.is_empty() {
        "/".into()
    } else {
        p
    }
}

fn dav_parent(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".into(),
        Some((parent, _)) => parent.to_string(),
    }
}

fn dav_is_dir(st: &State, path: &str) -> bool {
    path == "/" || st.dav_dirs.contains(path)
}

fn dav_remove(st: &mut State, path: &str) {
    let path = dav_path(path);
    let inside = format!("{path}/");
    st.dav.retain(|p, _| p != &path && !p.starts_with(&inside));
    st.dav_dirs.retain(|p| p != &path && !p.starts_with(&inside));
}

/// Адрес в ответе — как у Яндекс Диска: путь, закодированный по частям.
fn dav_href(path: &str, dir: bool) -> String {
    let mut out: String = path.split('/').filter(|s| !s.is_empty()).map(|s| format!("/{}", crate::http::enc(s))).collect();
    if dir {
        out.push('/');
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

fn dav_entry(path: &str, size: Option<usize>) -> String {
    let props = match size {
        None => "<d:resourcetype><d:collection/></d:resourcetype>".to_string(),
        Some(n) => format!("<d:resourcetype/><d:getcontentlength>{n}</d:getcontentlength>"),
    };
    format!("<d:response><d:href>{}</d:href><d:propstat><d:status>HTTP/1.1 200 OK</d:status><d:prop>{props}</d:prop></d:propstat></d:response>", dav_href(path, size.is_none()))
}

fn dav(req: &Req, st: &mut State) -> Out {
    let want = format!("{DAV_USER}:{}", st.dav_password.as_deref().unwrap_or(DAV_PASSWORD));
    let given = req.header("authorization").strip_prefix("Basic ").and_then(|b| B64.decode(b.trim()).ok());
    if given.as_deref() != Some(want.as_bytes()) {
        return raw(401, b"Unauthorized".to_vec());
    }
    if st.dav_forbid {
        return raw(403, b"Forbidden".to_vec());
    }
    let path = dav_path(&req.path["/dav".len()..]);
    let parent = dav_parent(&path);
    match req.method.as_str() {
        "PROPFIND" => {
            let mut body = String::from(r#"<?xml version='1.0' encoding='UTF-8'?><d:multistatus xmlns:d="DAV:">"#);
            if dav_is_dir(st, &path) {
                body.push_str(&dav_entry(&path, None));
                if req.header("depth") != "0" {
                    let inside = if path == "/" { "/".to_string() } else { format!("{path}/") };
                    let child = |p: &str| p.strip_prefix(&inside).is_some_and(|rest| !rest.is_empty() && !rest.contains('/'));
                    for d in st.dav_dirs.iter().filter(|d| child(d)) {
                        body.push_str(&dav_entry(d, None));
                    }
                    for (p, data) in st.dav.iter().filter(|(p, _)| child(p)) {
                        body.push_str(&dav_entry(p, Some(data.len())));
                    }
                }
            } else if let Some(data) = st.dav.get(&path) {
                body.push_str(&dav_entry(&path, Some(data.len())));
            } else {
                return raw(404, b"Not Found".to_vec());
            }
            body.push_str("</d:multistatus>");
            raw(207, body.into_bytes())
        }
        "MKCOL" => {
            if dav_is_dir(st, &path) || st.dav.contains_key(&path) {
                raw(405, b"Method Not Allowed".to_vec())
            } else if !dav_is_dir(st, &parent) {
                raw(409, b"Conflict".to_vec())
            } else {
                st.dav_dirs.insert(path);
                raw(201, Vec::new())
            }
        }
        "PUT" => {
            if !dav_is_dir(st, &parent) {
                raw(409, b"Conflict".to_vec())
            } else if dav_is_dir(st, &path) {
                raw(405, b"Method Not Allowed".to_vec())
            } else if st.full {
                raw(507, b"Insufficient Storage".to_vec())
            } else {
                let fresh = st.dav.insert(path, req.body.clone()).is_none();
                raw(if fresh { 201 } else { 204 }, Vec::new())
            }
        }
        "GET" => match st.dav.get(&path) {
            Some(d) => raw(200, d.clone()),
            None => raw(404, b"Not Found".to_vec()),
        },
        "DELETE" => {
            if dav_is_dir(st, &path) && path != "/" || st.dav.contains_key(&path) {
                dav_remove(st, &path);
                raw(204, Vec::new())
            } else {
                raw(404, b"Not Found".to_vec())
            }
        }
        _ => raw(405, b"Method Not Allowed".to_vec()),
    }
}

//! Запросы к облаку: таймауты, пределы ответа и понятные ошибки.

use kit_core::cloud::CloudFail;
use kit_core::{Error, Result};
use std::time::Duration;

/// Сейчас: секунды Unix.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub(crate) fn fail(kind: CloudFail, msg: impl Into<String>) -> Error {
    Error::Cloud(kind, msg.into())
}

pub(crate) fn no_space() -> Error {
    fail(CloudFail::Space, "В облаке не хватает места — освободите место на диске")
}

pub(crate) fn server(code: u16) -> Error {
    if code == 429 {
        return fail(CloudFail::Server, "Облако просит подождать: слишком много запросов. Обмен повторится позже");
    }
    fail(CloudFail::Server, format!("Облако ответило ошибкой {code} — обмен повторится позже"))
}

pub(crate) fn odd() -> Error {
    fail(CloudFail::Server, "Облако ответило непонятно — обмен повторится позже")
}

#[derive(Clone, Copy)]
pub(crate) enum Body<'a> {
    Empty,
    Bytes(&'a [u8]),
    /// Запрос WebDAV (PROPFIND).
    Xml(&'a [u8]),
}

pub(crate) struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

pub(crate) struct Http {
    agent: ureq::Agent,
}

fn net(e: ureq::Error) -> Error {
    match e {
        ureq::Error::BodyExceedsLimit(n) => fail(CloudFail::Format, format!("файл в облаке больше допустимого ({n} байт)")),
        e => fail(CloudFail::Offline, format!("Нет связи с облаком ({e}) — проверьте интернет. Обмен повторится позже")),
    }
}

impl Http {
    pub fn new() -> Http {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_resolve(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_send_request(Some(Duration::from_secs(60)))
            .timeout_recv_response(Some(Duration::from_secs(90)))
            // Обложка в несколько мегабайт по медленной мобильной связи идёт долго.
            .timeout_send_body(Some(Duration::from_secs(15 * 60)))
            .timeout_recv_body(Some(Duration::from_secs(15 * 60)))
            .user_agent("Polka")
            // WebDAV: PROPFIND и MKCOL.
            .allow_non_standard_methods(true)
            .build()
            .into();
        Http { agent }
    }

    /// Запрос; ответ — не больше `max` байт.
    pub fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Body, max: usize) -> Result<Resp> {
        let r = match method {
            "GET" | "DELETE" => {
                let mut b = if method == "GET" { self.agent.get(url) } else { self.agent.delete(url) };
                for (k, v) in headers {
                    b = b.header(*k, *v);
                }
                b.call()
            }
            "PUT" => {
                let mut b = self.agent.put(url);
                for (k, v) in headers {
                    b = b.header(*k, *v);
                }
                match body {
                    Body::Empty => b.send_empty(),
                    Body::Bytes(d) => b.header("Content-Type", "application/octet-stream").send(d),
                    Body::Xml(d) => b.header("Content-Type", "application/xml; charset=utf-8").send(d),
                }
            }
            // WebDAV: PROPFIND, MKCOL.
            _ => {
                let mut b = ureq::http::Request::builder().method(method).uri(url);
                for (k, v) in headers {
                    b = b.header(*k, *v);
                }
                let data: &[u8] = match body {
                    Body::Empty => &[],
                    Body::Xml(d) => {
                        b = b.header("Content-Type", "application/xml; charset=utf-8");
                        d
                    }
                    Body::Bytes(d) => d,
                };
                let req = b.body(data).map_err(|_| fail(CloudFail::Format, "Неверный адрес облака"))?;
                self.agent.run(req)
            }
        };
        let mut resp = r.map_err(net)?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(max as u64).read_to_vec().map_err(net)?;
        Ok(Resp { status, body })
    }
}

/// Кодирование для адреса: всё, кроме латинских букв, цифр и `-_.~`.
pub(crate) fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths() {
        assert_eq!(enc("aaaa-1-3.pev"), "aaaa-1-3.pev");
        assert_eq!(enc("Полка"), "%D0%9F%D0%BE%D0%BB%D0%BA%D0%B0");
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }
}

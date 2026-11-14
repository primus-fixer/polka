//! Запросы к каталогам: таймауты, предел размера ответа и понятные ошибки.
//! Через трейт `Http` каталоги проверяются в тестах без сети.

use crate::error::{Error, Result};
use std::time::Duration;

pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.body).map_err(|_| Error::Net("каталог ответил непонятно".into()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub trait Http: Sync {
    fn get(&self, url: &str, headers: &[(&str, &str)], max: usize) -> Result<Resp>;
}

/// Как «Полка» представляется каталогам.
pub const USER_AGENT: &str = "Polka/0.1 (personal book and film tracker)";

/// Настоящая сеть (ureq + rustls).
pub struct Net {
    agent: ureq::Agent,
}

impl Net {
    pub fn new() -> Net {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_resolve(Some(Duration::from_secs(10)))
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(20)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
            .build()
            .into();
        Net { agent }
    }
}

impl Default for Net {
    fn default() -> Self {
        Net::new()
    }
}

fn net(e: ureq::Error) -> Error {
    match e {
        ureq::Error::BodyExceedsLimit(_) => Error::Net("ответ каталога слишком большой".into()),
        other => Error::Net(format!("нет связи с каталогом: {other}")),
    }
}

impl Http for Net {
    fn get(&self, url: &str, headers: &[(&str, &str)], max: usize) -> Result<Resp> {
        let mut b = self.agent.get(url).header("User-Agent", USER_AGENT);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        let mut resp = b.call().map_err(net)?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(max as u64).read_to_vec().map_err(net)?;
        Ok(Resp { status, body })
    }
}

/// Кодирование строки для адреса (query): всё, кроме букв, цифр и `-_.~`, — в %XX.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(crate) fn status_error(source: &str, status: u16) -> Error {
    match status {
        429 => Error::Net(format!("{source}: слишком много запросов, попробуйте через минуту")),
        500..=599 => Error::Net(format!("{source}: каталог временно не работает ({status})")),
        _ => Error::Net(format!("{source}: ответ {status}")),
    }
}

#[cfg(test)]
pub mod fake {
    //! Подставная сеть для тестов: адрес → ответ.
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Fake {
        pub routes: HashMap<String, (u16, Vec<u8>)>,
        pub seen: Mutex<Vec<String>>,
    }

    impl Fake {
        pub fn route(mut self, prefix: &str, status: u16, body: &str) -> Fake {
            self.routes.insert(prefix.to_string(), (status, body.as_bytes().to_vec()));
            self
        }

        pub fn route_bytes(mut self, prefix: &str, status: u16, body: Vec<u8>) -> Fake {
            self.routes.insert(prefix.to_string(), (status, body));
            self
        }
    }

    impl Http for Fake {
        fn get(&self, url: &str, _headers: &[(&str, &str)], _max: usize) -> Result<Resp> {
            self.seen.lock().unwrap().push(url.to_string());
            let best = self.routes.iter().filter(|(p, _)| url.starts_with(p.as_str())).max_by_key(|(p, _)| p.len());
            match best {
                Some((_, (status, body))) => Ok(Resp { status: *status, body: body.clone() }),
                None => Err(Error::Net(format!("нет связи: {url}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_cyrillic() {
        assert_eq!(enc("пикник на"), "%D0%BF%D0%B8%D0%BA%D0%BD%D0%B8%D0%BA%20%D0%BD%D0%B0");
        assert_eq!(enc("isbn:978-5"), "isbn%3A978-5");
    }
}

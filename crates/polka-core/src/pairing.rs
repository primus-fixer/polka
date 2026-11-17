//! Связь двух устройств: код связи и QR-код — как в «Кармане» 0.15.
//!
//! Одно устройство показывает код из восьми знаков (и QR-код с тем же кодом и своим адресом),
//! другое вводит код или сканирует QR. Из кода оба получают общий ключ по SPAKE2 ([`crate::lan`]):
//! сам код по сети не передаётся, а подобрать его можно только попытками на самом устройстве — их пять.
//!
//! Адреса в QR — только адреса домашней сети: QR-код чужого с адресом в интернете
//! не уведёт записи на чужой сервер.

use crate::error::{bad, Error, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const SCHEME: &str = "polka://pair?";

pub use kit_core::lan::{is_local_address, show_code, CODE_ALPHABET, CODE_LEN};

/// Новый код связи: восемь случайных знаков (из общего модуля kit).
pub fn new_code() -> Result<String> {
    Ok(kit_core::lan::new_code()?)
}

/// Код, как его ввёл человек, — к виду «6WVGHEWK» (из общего модуля kit): без пробелов и дефисов,
/// заглавными, русские буквы, похожие на латинские, — латинскими.
pub fn normalize_code(input: &str) -> Result<String> {
    Ok(kit_core::lan::normalize_code(input)?)
}

/// QR-код связи: адрес устройства в сети и код связи. Ключа в нём нет.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Invite {
    pub device: String,
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub code: String,
}

impl Invite {
    pub fn to_uri(&self) -> String {
        format!("{SCHEME}v=1&d={}&n={}&h={}&p={}&c={}", self.device, B64.encode(self.name.as_bytes()), self.hosts.join(","), self.port, self.code)
    }
}

/// Разбирает QR-код связи.
pub fn parse_qr(uri: &str) -> Result<Invite> {
    let t = uri.trim();
    let Some(q) = t.strip_prefix(SCHEME) else {
        return Err(bad("это не QR-код «Полки» для связи"));
    };
    let mut fields = std::collections::BTreeMap::new();
    for part in q.split('&') {
        let (k, v) = part.split_once('=').ok_or_else(|| bad("испорченный QR-код связи"))?;
        fields.insert(k, v);
    }
    if fields.get("v").copied() != Some("1") {
        return Err(bad("QR-код от более новой версии приложения: обновите «Полку» на этом устройстве"));
    }
    let name = match fields.get("n") {
        Some(v) => String::from_utf8(B64.decode(v).map_err(|_| bad("имя устройства не читается"))?).map_err(|_| bad("имя устройства не читается"))?,
        None => String::new(),
    };
    let hosts: Vec<String> = fields.get("h").map(|h| h.split(',').filter(|h| !h.is_empty()).map(String::from).collect()).unwrap_or_default();
    if hosts.is_empty() {
        return Err(bad("в QR нет адреса"));
    }
    let hosts: Vec<String> = hosts.into_iter().filter(|h| is_local_address(h)).collect();
    if hosts.is_empty() {
        return Err(bad("в QR-коде адрес не из домашней сети: связь — только между устройствами в одной сети Wi-Fi"));
    }
    let device = fields.get("d").filter(|d| !d.is_empty() && d.len() <= 64).ok_or_else(|| bad("в QR нет устройства"))?.to_string();
    let port = fields.get("p").and_then(|p| p.parse::<u16>().ok()).ok_or_else(|| bad("в QR нет порта"))?;
    let code = normalize_code(fields.get("c").copied().unwrap_or(""))?;
    Ok(Invite { device, name, hosts, port, code })
}

pub fn random_key() -> Result<[u8; 32]> {
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).map_err(|e| Error::Random(e.to_string()))?;
    Ok(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_home_network_addresses() {
        for ok in ["192.168.1.20", "10.0.0.5", "172.20.10.2", "192.168.43.1", "100.101.1.2", "169.254.3.4", "127.0.0.1", "fe80::1", "fd12:3456::1", "::1"] {
            assert!(is_local_address(ok), "{ok}");
        }
        for bad in ["8.8.8.8", "93.184.216.34", "172.32.0.1", "100.128.0.1", "evil.example.com", "localhost", "2001:db8::1", ""] {
            assert!(!is_local_address(bad), "{bad}");
        }
    }

    #[test]
    fn codes_are_random_readable_and_forgiving() {
        let a = new_code().unwrap();
        let b = new_code().unwrap();
        assert_eq!(a.len(), CODE_LEN);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| CODE_ALPHABET.contains(&c)), "{a}");
        assert_eq!(show_code("6WVGHEWK"), "6WVG-HEWK");
        assert_eq!(normalize_code(" 6wvg-hewk ").unwrap(), "6WVGHEWK");
        // С русской раскладки: «Н», «Е», «В», «К», «Р», «С», «Т», «М», «Х», «А» — как латинские.
        assert_eq!(normalize_code("НЕВК–РСТМ").unwrap(), "HEBKPCTM");
        assert_eq!(normalize_code("ха3у ксам").unwrap(), "XA3YKCAM");
        assert!(normalize_code("6WVG-HEW").unwrap_err().to_string().contains("восемь знаков"));
        assert!(normalize_code("6WVG-HEW0").unwrap_err().to_string().contains("«0»"), "нуля в коде не бывает");
        assert!(normalize_code("6WVG-HEWЖ").unwrap_err().to_string().contains("«Ж»"));
    }

    #[test]
    fn invite_qr_roundtrip() {
        let inv = Invite { device: "a1b2".into(), name: "Ноутбук".into(), hosts: vec!["192.168.1.20".into(), "8.8.8.8".into()], port: 47_510, code: "6WVGHEWK".into() };
        assert!(inv.to_uri().starts_with("polka://pair?v=1&d=a1b2&"));
        let back = parse_qr(&inv.to_uri()).unwrap();
        assert_eq!(back, Invite { hosts: vec!["192.168.1.20".into()], ..inv.clone() }, "адрес из интернета отброшен");
        let only_internet = Invite { hosts: vec!["8.8.8.8".into()], ..inv.clone() };
        assert!(parse_qr(&only_internet.to_uri()).unwrap_err().to_string().contains("не из домашней сети"));
        let bad_code = inv.to_uri().replace("c=6WVGHEWK", "c=123");
        assert!(parse_qr(&bad_code).is_err());
        assert!(parse_qr("polka://pair?v=2&d=x").unwrap_err().to_string().contains("более новой версии"));
        assert!(parse_qr("karman://pair?v=2&d=x&h=192.168.1.2&p=1&c=6WVGHEWK").unwrap_err().to_string().contains("не QR-код «Полки»"));
        assert!(parse_qr("https://example.com").is_err());
    }
}

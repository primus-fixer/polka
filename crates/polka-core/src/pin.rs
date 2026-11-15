//! PIN-код входа: формат, хеш Argon2id и паузы после неверных попыток.
//!
//! PIN у каждого устройства свой и хранится в локальных настройках базы (meta),
//! в журнал синхронизации не попадает. Ключ базы защищён хранилищем системы,
//! а PIN не даёт открыть приложение тому, кто взял разблокированное устройство.

use crate::error::{Error, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Длина PIN-кода.
pub const PIN_LEN: usize = 4;
/// Сколько ошибок подряд можно сделать без паузы.
pub const FREE_TRIES: u32 = 5;

/// Ключи в meta.
pub const META_HASH: &str = "pin.hash";
pub const META_FAILS: &str = "pin.fails";
pub const META_UNTIL: &str = "pin.until";
pub const META_BIO: &str = "pin.bio";

/// Ровно четыре цифры.
pub fn valid(pin: &str) -> bool {
    pin.len() == PIN_LEN && pin.bytes().all(|b| b.is_ascii_digit())
}

pub fn hash(pin: &str) -> Result<String> {
    if !valid(pin) {
        return Err(Error::Pin(format!("нужно {PIN_LEN} цифры")));
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| Error::Random(e.to_string()))?;
    let salt = SaltString::encode_b64(&salt).map_err(|e| Error::Pin(e.to_string()))?;
    Argon2::default().hash_password(pin.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| Error::Pin(e.to_string()))
}

pub fn verify(pin: &str, stored: &str) -> bool {
    valid(pin) && PasswordHash::new(stored).map(|h| Argon2::default().verify_password(pin.as_bytes(), &h).is_ok()).unwrap_or(false)
}

/// Пауза в секундах после `fails` неверных попыток подряд: первые пять — без паузы,
/// дальше каждая ошибка удлиняет ожидание.
pub fn pause_after(fails: u32) -> u64 {
    match fails {
        0..FREE_TRIES => 0,
        5 => 30,
        6 => 60,
        7 => 300,
        8 => 900,
        _ => 3600,
    }
}

/// Сколько ещё можно ошибиться до паузы.
pub fn tries_left(fails: u32) -> u32 {
    FREE_TRIES.saturating_sub(fails)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format() {
        assert!(valid("0420"));
        assert!(!valid("042"));
        assert!(!valid("04201"));
        assert!(!valid("04a0"));
        assert!(!valid("٠١٢٣"), "только цифры ASCII");
        assert!(hash("12").is_err());
    }

    #[test]
    fn hash_and_verify() {
        let h = hash("2580").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("2580", &h));
        assert!(!verify("2581", &h));
        assert!(!verify("2580", "мусор"));
        assert_ne!(hash("2580").unwrap(), h, "соль у каждого хеша своя");
    }

    #[test]
    fn pauses_grow() {
        assert_eq!((0..5).map(pause_after).sum::<u64>(), 0);
        assert_eq!(pause_after(5), 30);
        assert!(pause_after(6) > pause_after(5));
        assert!(pause_after(8) > pause_after(7));
        assert_eq!(pause_after(40), 3600);
        assert_eq!(tries_left(0), 5);
        assert_eq!(tries_left(4), 1);
        assert_eq!(tries_left(7), 0);
    }
}

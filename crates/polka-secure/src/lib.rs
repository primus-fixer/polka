//! Ключ базы «Полки» в хранилище системы и проверка, что за устройством владелец.
//!
//! - Windows: ключ шифруется учётной записью пользователя Windows (DPAPI);
//!   проверка личности — Windows Hello (отпечаток, лицо или PIN Windows).
//! - Android: ключ шифруется ключом AES из Android Keystore, который не покидает телефон.
//!   Отпечаток на Android — через плагин biometric в самом приложении.
//! - Остальные системы (Linux для разработки): ключ хранится как есть.

#[cfg(target_os = "android")]
mod android;
#[cfg(windows)]
mod win;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("хранилище ключей системы: {0}")]
    Store(String),
    #[error("защищённый ключ повреждён")]
    Format,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Чем защищён ключ на этой системе: `dpapi`, `android-keystore` или `file`.
pub fn method() -> &'static str {
    if cfg!(windows) {
        "dpapi"
    } else if cfg!(target_os = "android") {
        "android-keystore"
    } else {
        "file"
    }
}

/// Можно ли защитить ключ средствами системы.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "android"))
}

/// Шифрует секрет так, что расшифровать его можно только на этом устройстве
/// (на Windows — ещё и только под этой учётной записью).
pub fn protect(secret: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    return win::protect(secret);
    #[cfg(target_os = "android")]
    return android::protect(secret);
    #[cfg(not(any(windows, target_os = "android")))]
    Ok(secret.to_vec())
}

pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    return win::unprotect(blob);
    #[cfg(target_os = "android")]
    return android::unprotect(blob);
    #[cfg(not(any(windows, target_os = "android")))]
    Ok(blob.to_vec())
}

/// Настроен ли Windows Hello (отпечаток, лицо или PIN Windows).
pub fn hello_available() -> bool {
    #[cfg(windows)]
    return win::hello_available();
    #[cfg(not(windows))]
    false
}

/// Просит Windows Hello подтвердить личность. Окно запроса — поверх окна `hwnd`.
/// Блокирует поток до ответа, поэтому вызывать не из главного потока.
pub fn hello_verify(hwnd: isize, message: &str) -> Result<bool> {
    #[cfg(windows)]
    return win::hello_verify(hwnd, message);
    #[cfg(not(windows))]
    {
        let _ = (hwnd, message);
        Err(Error::Store("Windows Hello есть только в Windows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_on_this_system() {
        let secret = [7u8; 32];
        let blob = protect(&secret).unwrap();
        assert_eq!(unprotect(&blob).unwrap(), secret);
    }
}

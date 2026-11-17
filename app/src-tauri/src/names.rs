//! Чем «Полка» отличается от других приложений на общем модуле kit: название для сообщений
//! и строки, которыми ключи привязаны к «Полке».
//!
//! **Строки ключей менять нельзя.** С другими строками система не расшифрует ключ базы, пароль
//! облака и ключи связи, которые уже лежат на устройствах, — «Полка» не откроет свою базу.

use kit_core::AppNames;
use kit_secure::KeyNames;

pub const APP: AppNames = AppNames { id: "polka", name: "Полка", name_acc: "Полку", name_gen: "Полки", feminine: true };

pub const KEYS: KeyNames = KeyNames { dpapi_entropy: b"polka-db-key-v1", dpapi_label: "Polka", keystore_alias: "polka-db-key" };

/// Шифрует секрет средствами системы: DPAPI на Windows, Android Keystore на телефоне;
/// на Linux (разработка) секрет остаётся как есть.
pub fn protect(secret: &[u8]) -> kit_secure::Result<Vec<u8>> {
    kit_secure::protect(&KEYS, secret)
}

pub fn unprotect(blob: &[u8]) -> kit_secure::Result<Vec<u8>> {
    kit_secure::unprotect(&KEYS, blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Строки — те же, что были в «Полке» до общего модуля (0.11.0). Проверка падает, если их
    /// поменять: тогда на устройствах перестанут открываться база, облако и связанные устройства.
    #[test]
    fn key_names_are_the_same_as_before_kit() {
        assert_eq!(KEYS.dpapi_entropy, b"polka-db-key-v1");
        assert_eq!(KEYS.dpapi_label, "Polka");
        assert_eq!(KEYS.keystore_alias, "polka-db-key");
        // Имена переменных окружения для сквозных проверок — тоже прежние.
        assert_eq!(APP.env("LOCK_AFTER_SECS"), "POLKA_LOCK_AFTER_SECS");
        assert_eq!(APP.env("FAKE_AUTH"), "POLKA_FAKE_AUTH");
    }

    #[test]
    fn secret_roundtrip() {
        let blob = protect(&[7u8; 32]).unwrap();
        assert_eq!(unprotect(&blob).unwrap(), [7u8; 32]);
    }
}

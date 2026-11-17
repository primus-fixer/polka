//! Облако: Яндекс Диск по логину и паролю приложения или свой сервер WebDAV. Пароль копий и обмен в фоне.
//!
//! С 0.12 всё это — общее, из kit (`kit_tauri::cloud`), то же, что у «Кармана»: вход, пароль копий,
//! обмен в фоне и команды экрана. Своё у «Полки» — формат облака (`polka_core::cloud::FORMAT`), база
//! и где что лежит в ней: пароль сервера — в `cloud.password` под защитой системы.

use crate::Res;
use polka_core::cloud::{KeyFile, FORMAT};
use polka_core::db::Store;
use tauri::AppHandle;

kit_tauri::cloud_commands!(Store);

/// Облако в приложении — состояние для `app.manage`.
pub type Sky = kit_tauri::cloud::Sky<Store>;

/// Где что лежит у «Полки». Менять нельзя: вход в облако на устройствах пропадёт.
pub fn settings() -> kit_tauri::cloud::Settings {
    kit_tauri::cloud::Settings { cloud: &FORMAT, backup: &polka_core::backup::BACKUP, keys: &crate::names::KEYS, secret: "cloud.password", backup_key: crate::backup::META_KEY }
}

/// Обмен в фоне.
pub fn start(app: AppHandle) {
    kit_tauri::cloud::start::<_, Store>(app);
}

/// Вернулись в «Полку»: обменяться, если давно не обменивались.
pub fn resumed() {
    kit_tauri::cloud::resumed();
}

/// Ключ облака на этом устройстве, если облако подключено.
pub fn key_meta(st: &Store) -> Option<String> {
    kit_tauri::cloud::key_meta(st)
}

/// Пароль копий сменили: тот же ключ облака под новым паролем (около секунды).
pub fn rewrap(key_meta: &str, password: &str) -> Res<KeyFile> {
    kit_tauri::cloud::rewrap(&FORMAT, key_meta, password)
}

/// Новый ключ облака уйдёт в облако при ближайшем обмене.
pub fn save_keyfile(st: &Store, kf: &KeyFile) -> Res<()> {
    kit_tauri::cloud::save_keyfile(st, kf)
}

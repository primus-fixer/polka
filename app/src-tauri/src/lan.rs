//! Связь по Wi-Fi: связанные устройства, код связи, поиск в сети и обмен в фоне.
//!
//! С 0.12 всё это — общее, из kit (`kit_tauri::wifi`), то же, что у «Кармана»: команды экрана, фоновый
//! обмен и хранение связанных устройств. Своё у «Полки» — настройки связи (`polka_core::lan::LAN`),
//! база и строки ключей для хранилища системы.

use polka_core::db::Store;
use tauri::AppHandle;

kit_tauri::wifi_commands!(Store);

/// Связь по Wi-Fi в приложении — состояние для `app.manage`.
pub type Wifi = kit_tauri::wifi::Wifi<Store>;

/// Фон: открыть порты, когда нужно, находить связанные устройства и обмениваться.
pub fn start(app: AppHandle) {
    kit_tauri::wifi::start::<_, Store>(app);
}

/// «Полка» заблокировалась: код связи больше не действует.
pub fn on_lock(app: &AppHandle) {
    kit_tauri::wifi::on_lock::<_, Store>(app);
}

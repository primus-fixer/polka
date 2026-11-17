//! WebDAV: Яндекс Диск по логину и паролю приложения или свой сервер (Nextcloud, NAS).
//!
//! «Полка» работает только в папке «Полка» в корне диска. Пароль приложения открывает весь Диск,
//! поэтому он хранится только на устройстве, под защитой системы. Файлы в папке зашифрованы ядром
//! (`polka_core::cloud`): без пароля копий их не прочитать.
//!
//! С 0.11.9 сам клиент — общий, из kit (`kit_cloud::dav`), тот же, что у «Кармана»; здесь — папка
//! и файлы «Полки» (`polka_core::cloud::FORMAT`).

use kit_core::Result;
use polka_core::cloud::{Cloud, Dir, Entry, FORMAT};

pub use kit_cloud::dav::{host_of, is_yandex, server_url, YANDEX};

/// Папка «Полки» в корне диска.
pub const FOLDER: &str = "Полка";

/// Подключение к серверу WebDAV для одного обмена.
pub struct Dav(kit_cloud::dav::Dav);

impl Dav {
    /// `server` — адрес, проверенный [`server_url`].
    pub fn new(server: &str, user: &str, password: &str) -> Dav {
        Dav(kit_cloud::dav::Dav::new(&FORMAT, server, user, password))
    }

    /// Проверяет вход и создаёт папку «Полка», если её нет. Возвращает, есть ли в ней данные «Полки».
    pub fn connect(&self) -> Result<bool> {
        self.0.connect()
    }

    /// После удаления данных из облака: убирает пустые папки «Полки». Чужие файлы не трогает.
    pub fn tidy(&self) -> Result<()> {
        self.0.tidy()
    }
}

impl Cloud for Dav {
    fn list(&self, dir: Dir) -> Result<Vec<Entry>> {
        self.0.list(dir)
    }
    fn get(&self, dir: Dir, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        self.0.get(dir, name, max)
    }
    fn put(&self, dir: Dir, name: &str, data: &[u8]) -> Result<()> {
        self.0.put(dir, name, data)
    }
    fn exists(&self, dir: Dir, name: &str) -> Result<bool> {
        self.0.exists(dir, name)
    }
    fn delete(&self, dir: Dir, name: &str) -> Result<()> {
        self.0.delete(dir, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_is_the_app_name() {
        assert_eq!(FOLDER, FORMAT.names.name);
    }
}

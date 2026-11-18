//! Облако «Полки»: Яндекс Диск по логину и паролю приложения или свой сервер WebDAV
//! (Nextcloud, NAS). Как в «Кармане»: тот же код, своя папка «Полка» в корне диска.
//!
//! В облако уходят файлы, уже зашифрованные ядром (`polka_core::cloud`). Клиент WebDAV — общий, из kit (`kit_cloud`).

pub mod dav;
#[cfg(any(test, feature = "mock"))]
pub mod mock;

pub use kit_cloud::now;

#[cfg(test)]
mod tests;

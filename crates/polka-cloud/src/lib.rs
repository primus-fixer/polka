//! Облако «Полки»: Яндекс Диск по логину и паролю приложения или свой сервер WebDAV
//! (Nextcloud, NAS). Как в «Кармане»: тот же код, своя папка «Полка» в корне диска.
//!
//! В облако уходят файлы, уже зашифрованные ядром (`polka_core::cloud`).

pub mod dav;
mod http;
#[cfg(any(test, feature = "mock"))]
pub mod mock;

pub use http::now;

#[cfg(test)]
mod tests;

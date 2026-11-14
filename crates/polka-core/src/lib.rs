//! Ядро «Полки»: книги, фильмы и сериалы, каталоги и обложки, зашифрованная база.
//!
//! - `model` — запись о книге, фильме или сериале: статус, даты, оценка, метки, сезоны и серии;
//! - `db` — зашифрованная база (SQLCipher) с журналом событий, как в «Кармане»;
//! - `catalog` — поиск в Open Library, Google Books, FantLab и ПоискКино, добавление по ссылке,
//!   варианты обложек и скачивание выбранной;
//! - `stats` — итоги за год: цель, страницы и часы, жанры, авторы, оценки;
//! - `backup` — резервная копия: журнал и обложки в одном файле, зашифрованном паролем копии;
//! - `sync`, `pairing`, `lan` — обмен по Wi-Fi: связь по коду (SPAKE2), поиск в сети, обмен записями и обложками.
//! - `cloud` — обмен через облако (Яндекс Диск, свой сервер WebDAV): зашифрованные файлы событий и обложек.
//! - `reader` — читалка: книги EPUB и FB2, главы для страницы, оглавление, поиск, место чтения.

pub mod backup;
pub mod catalog;
pub mod cloud;
pub mod date;
pub mod db;
pub mod error;
pub mod files;
pub mod isbn;
pub mod lan;
pub mod model;
pub mod pairing;
pub mod pin;
pub mod reader;
pub mod stats;
pub mod sync;

pub use date::Date;
pub use error::{Error, Result};

/// Новый номер записи: 16 шестнадцатеричных знаков.
pub fn new_id() -> Result<String> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| Error::Random(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Сейчас: секунды Unix.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

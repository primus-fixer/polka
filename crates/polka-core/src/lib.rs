//! Ядро «Полки»: книги, фильмы и сериалы, каталоги и обложки, зашифрованная база.
//!
//! - `model` — запись о книге, фильме или сериале: статус, даты, оценка, метки, сезоны и серии;
//! - `db` — зашифрованная база (SQLCipher) с журналом событий, как в «Кармане»;
//! - `catalog` — поиск в Open Library, Google Books, FantLab и ПоискКино, добавление по ссылке,
//!   варианты обложек и скачивание выбранной;
//! - `stats` — итоги за год: цель, страницы и часы, жанры, авторы, оценки;
//! - `backup` — резервная копия: журнал и обложки в одном файле, зашифрованном паролем копии.

pub mod backup;
pub mod catalog;
pub mod date;
pub mod db;
pub mod error;
pub mod isbn;
pub mod model;
pub mod stats;

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

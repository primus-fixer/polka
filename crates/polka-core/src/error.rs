//! Ошибки ядра — сразу понятным текстом для экрана.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("база: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("данные: {0}")]
    Json(#[from] serde_json::Error),
    #[error("файл: {0}")]
    Io(#[from] std::io::Error),
    #[error("неверный ключ базы")]
    WrongKey,
    /// Неверные данные от пользователя или каталога.
    #[error("{0}")]
    Format(String),
    /// Каталог недоступен или ответил ошибкой.
    #[error("{0}")]
    Net(String),
    #[error("не найдено: {0}")]
    NotFound(String),
    #[error("случайные числа недоступны: {0}")]
    Random(String),
    /// Резервная копия: файл не подходит, повреждён или обрезан.
    #[error("{0}")]
    Backup(String),
    #[error("Пароль не подходит к этой копии")]
    BackupPassword,
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn bad(msg: impl Into<String>) -> Error {
    Error::Format(msg.into())
}

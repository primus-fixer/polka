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
    /// Пакет обмена не расшифровался: чужой ключ, подмена или не на своём месте.
    #[error("пакет не расшифровался")]
    Decrypt,
    /// Другое устройство не смогло принять записи или ответило не так.
    #[error("{0}")]
    Peer(String),
    /// Связь по Wi-Fi: что случилось и что сказать.
    #[error("{1}")]
    Lan(LanFail, String),
}

/// Почему не получилась связь по Wi-Fi — чтобы приложение могло ответить по делу.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanFail {
    /// На том устройстве код не показан, истёк или уже использован.
    NoCode,
    /// Код не подошёл; сколько попыток осталось.
    WrongCode(u8),
    /// Код отменён после пяти неверных попыток.
    Cancelled,
    /// Устройство не узнало нас: связь отвязана или её не было.
    Unknown,
    /// Устройство занято другим обменом.
    Busy,
    /// Ответило не то устройство: оно не знает кода или ключа пары.
    NotPeer,
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn bad(msg: impl Into<String>) -> Error {
    Error::Format(msg.into())
}

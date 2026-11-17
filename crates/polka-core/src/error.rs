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
    /// Облако: что случилось и что сказать.
    #[error("{1}")]
    Cloud(CloudFail, String),
    #[error("Пароль не подходит: облако зашифровано другим паролем копий")]
    CloudPassword,
}

/// Что случилось с облаком — от этого зависит подсказка на экране.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudFail {
    /// Логин или пароль не подходит, доступ отозван — войти заново.
    Auth,
    /// На диске нет места.
    Space,
    /// Нет связи с облаком.
    Offline,
    /// Облако ответило ошибкой — обычно временной.
    Server,
    /// Данные «Полки» в облаке удалены.
    Gone,
    /// Облако подключили заново, с другим ключом.
    Replaced,
    /// Записи этого устройства в облаке расходятся с базой.
    Conflict,
    /// Файл записан более новой версией «Полки».
    Newer,
    /// Файл или ответ облака не разобрать.
    Format,
    /// Обмен остановлен.
    Cancelled,
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

/// Ошибки общего модуля kit — теми же словами, что и раньше в «Полке».
impl From<kit_core::Error> for Error {
    fn from(e: kit_core::Error) -> Self {
        use kit_core::Error as K;
        match e {
            K::Backup(s) => Error::Backup(s),
            K::BackupPassword => Error::BackupPassword,
            K::Random(s) => Error::Random(s),
            K::Io(e) => Error::Io(e),
            K::Json(e) => Error::Json(e),
            other => Error::Format(other.to_string()),
        }
    }
}

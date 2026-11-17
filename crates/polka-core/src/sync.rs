//! Обмен записями между устройствами — общий для всех каналов (Wi-Fi, позже — облако).
//!
//! Протокол — общий из kit (`kit_core::sync`), тот же, что у «Кармана»: каждое устройство знает последний
//! номер события от каждого устройства (вектор версий). При обмене стороны называют свои векторы, отдают
//! друг другу недостающие события и потом — недостающие снимки обложек (по номерам из записей). Повторы
//! пропускаются, при споре побеждает более позднее изменение (это делает база).
//!
//! С 0.8.0 после обложек — файлы книг (если обмен ими включён на обоих устройствах): каждая сторона
//! забирает у другой недостающие файлы кусками, с докачкой (см. [`crate::files`]). Здесь — то, чем
//! «Полка» отличается: её журнал ([`Journal`] для базы), файлы книг и настройки обмена.
//!
//! Канал шифрует каждое сообщение; ключи и порядок пакетов — в общем модуле (`kit_core::lan`),
//! настройки «Полки» — в [`crate::lan`].

use crate::db::{Event, Store};
use crate::error::Error;
use crate::files::{self, BookFile};
use kit_core::sync::{self as kit, FileInfo, Journal, Profile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Последний известный номер события от каждого устройства.
pub type VersionVector = BTreeMap<String, i64>;

/// Версия «Полки»: другое устройство узнаёт её при обмене.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Обмен «Полки»: поля `files` нет (вложения умеют все её версии), файлы книг — с 0.8.0.
pub const PROFILE: Profile = Profile { version: APP_VERSION, files_flag: false, old_peer: "", chunked_since: Some((0, 8)) };

/// Чем закончился обмен.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SyncReport {
    pub peer: String,
    pub peer_name: String,
    /// Сколько событий ушло другому устройству и было им принято.
    pub sent: usize,
    /// Сколько новых событий пришло к нам.
    pub received: usize,
    /// Сколько обложек ушло и пришло.
    pub files_sent: usize,
    pub files_received: usize,
    /// Версия «Полки» на другом устройстве.
    pub peer_app: String,
    /// Сколько файлов книг ушло и пришло целиком.
    #[serde(default)]
    pub books_sent: usize,
    #[serde(default)]
    pub books_received: usize,
}

impl From<kit::SyncReport> for SyncReport {
    fn from(r: kit::SyncReport) -> Self {
        SyncReport { peer: r.peer, peer_name: r.peer_name, sent: r.sent, received: r.received, files_sent: r.files_sent, files_received: r.files_received, peer_app: r.peer_app, books_sent: r.whole_sent, books_received: r.whole_received }
    }
}

/// Файлы книг по Wi-Fi — с 0.8.0: со старой «Полкой» эту часть обмена пропускаем.
#[cfg(test)]
pub(crate) fn speaks_files(app: &str) -> bool {
    let mut it = app.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    let (major, minor) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    major > 0 || minor >= 8
}

impl FileInfo for BookFile {
    fn id(&self) -> &str {
        &self.id
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn store_err(e: Error) -> kit_core::Error {
    kit_core::Error::Store(e.to_string())
}

/// Журнал «Полки» для общего протокола: события, обложки и файлы книг.
impl Journal for Store {
    type Event = Event;
    type File = BookFile;

    fn device(&self) -> String {
        Store::device(self).to_string()
    }
    fn versions(&self) -> kit_core::Result<kit::VersionVector> {
        Ok(Store::versions(self).map_err(store_err)?.into_iter().map(|(d, s)| (d, s.max(0) as u64)).collect())
    }
    fn events_missing(&self, theirs: &kit::VersionVector) -> kit_core::Result<Vec<Event>> {
        let theirs: VersionVector = theirs.iter().map(|(d, s)| (d.clone(), (*s).min(i64::MAX as u64) as i64)).collect();
        Store::events_missing(self, &theirs).map_err(store_err)
    }
    fn apply_remote(&mut self, events: &[Event]) -> kit_core::Result<usize> {
        Store::apply_remote(self, events).map_err(store_err)
    }
    fn missing_blobs(&self) -> kit_core::Result<Vec<String>> {
        Store::missing_blobs(self).map_err(store_err)
    }
    fn blob(&self, id: &str) -> kit_core::Result<Option<(String, Vec<u8>)>> {
        Store::blob(self, id).map_err(store_err)
    }
    fn put_blob_checked(&mut self, id: &str, mime: &str, data: &[u8]) -> kit_core::Result<()> {
        Store::put_blob_checked(self, id, mime, data).map(|_| ()).map_err(store_err)
    }
    fn files_on(&self) -> bool {
        files::sync_on(self)
    }
    fn files_wanted(&self) -> kit_core::Result<Vec<BookFile>> {
        files::wanted(self).map_err(store_err)
    }
    fn files_present(&self) -> kit_core::Result<Vec<BookFile>> {
        files::present(self).map_err(store_err)
    }
    fn file_partial(&self, id: &str) -> u64 {
        files::partial(self, id)
    }
    fn file_read(&self, id: &str, offset: u64) -> kit_core::Result<Option<(u64, Vec<u8>)>> {
        files::read_part(self, id, offset).map_err(store_err)
    }
    fn file_write(&mut self, file: &BookFile, offset: u64, total: u64, data: &[u8]) -> kit_core::Result<bool> {
        files::write_part(self, file, offset, total, data).map_err(store_err)
    }
}

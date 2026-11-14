//! Зашифрованная база (SQLCipher) с журналом событий — устроена как в «Кармане».
//!
//! Каждое изменение — событие с номером от устройства и часами Лэмпорта. Таблица записей
//! собирается из событий: при споре двух устройств побеждает более позднее изменение
//! (по часам Лэмпорта, при равенстве — по номеру устройства). Журнал ведётся с первого дня,
//! поэтому обмен по Wi-Fi и через облако можно включить позже без потери записей.
//! Снимки обложек лежат отдельно, по SHA-256 содержимого, и в журнал не попадают.

use crate::error::{bad, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Снимок обложки — не больше 8 МБ.
pub const MAX_BLOB: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum EventBody {
    Put { kind: String, id: String, data: serde_json::Value },
    Del { kind: String, id: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub device: String,
    pub seq: i64,
    pub lamport: i64,
    pub body: EventBody,
}

pub struct Store {
    conn: Connection,
    device: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Номер снимка — SHA-256 содержимого.
pub fn blob_id(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Тип картинки по первым байтам: под видом обложки не пролезет что-то другое.
/// Снимок подходит для базы: не пустой, не больше предела, тип совпадает с первыми байтами.
pub fn check_blob(mime: &str, data: &[u8]) -> Result<()> {
    if data.is_empty() || data.len() > MAX_BLOB {
        return Err(bad(format!("снимок пустой или больше {} МБ", MAX_BLOB / 1024 / 1024)));
    }
    if sniff_image(data) != Some(mime) {
        return Err(bad("подходят только снимки JPEG, PNG и WebP"));
    }
    Ok(())
}

pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

impl Store {
    /// Открывает или создаёт базу, зашифрованную 256-битным ключом.
    pub fn open(path: &Path, key: &[u8; 32], device: &str) -> Result<Store> {
        let conn = Connection::open(path)?;
        let mut pragma = format!("PRAGMA key = \"x'{}'\";", hex(key)).into_bytes();
        let keyed = conn.execute_batch(std::str::from_utf8(&pragma).unwrap_or_default());
        pragma.fill(0);
        std::hint::black_box(&pragma);
        keyed?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(|_| Error::WrongKey)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS events(
               device TEXT NOT NULL, seq INTEGER NOT NULL, lamport INTEGER NOT NULL, body TEXT NOT NULL,
               PRIMARY KEY(device, seq));
             CREATE TABLE IF NOT EXISTS items(
               kind TEXT NOT NULL, id TEXT NOT NULL, body TEXT, deleted INTEGER NOT NULL DEFAULT 0,
               lamport INTEGER NOT NULL, device TEXT NOT NULL,
               PRIMARY KEY(kind, id));
             CREATE TABLE IF NOT EXISTS blobs(
               id TEXT PRIMARY KEY, mime TEXT NOT NULL, size INTEGER NOT NULL, data BLOB NOT NULL,
               added INTEGER NOT NULL);",
        )?;
        let store = Store { conn, device: device.to_string() };
        if store.meta("schema")?.is_none() {
            store.set_meta("schema", "1")?;
        }
        Ok(store)
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    fn record(&mut self, body: EventBody) -> Result<()> {
        let device = self.device.clone();
        let tx = self.conn.transaction()?;
        let lamport: i64 = tx.query_row("SELECT COALESCE(MAX(lamport), 0) + 1 FROM events", [], |r| r.get(0))?;
        let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE device = ?1", [&device], |r| r.get(0))?;
        let ev = Event { device, seq, lamport, body };
        tx.execute(
            "INSERT INTO events(device, seq, lamport, body) VALUES(?1, ?2, ?3, ?4)",
            params![ev.device, ev.seq, ev.lamport, serde_json::to_string(&ev.body)?],
        )?;
        apply(&tx, &ev)?;
        tx.commit()?;
        Ok(())
    }

    /// Записать значение (новое или изменённое).
    pub fn put<T: Serialize>(&mut self, kind: &str, id: &str, value: &T) -> Result<()> {
        if kind.is_empty() || id.is_empty() {
            return Err(bad("пустой вид или номер записи"));
        }
        let data = serde_json::to_value(value)?;
        self.record(EventBody::Put { kind: kind.into(), id: id.into(), data })
    }

    pub fn delete(&mut self, kind: &str, id: &str) -> Result<()> {
        self.record(EventBody::Del { kind: kind.into(), id: id.into() })
    }

    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        let body: Option<Option<String>> = self
            .conn
            .query_row("SELECT body FROM items WHERE kind = ?1 AND id = ?2 AND deleted = 0", params![kind, id], |r| r.get(0))
            .optional()?;
        match body.flatten() {
            Some(b) => Ok(Some(serde_json::from_str(&b)?)),
            None => Ok(None),
        }
    }

    /// Все живые записи вида. Запись, которую эта версия не понимает, пропускается, а не роняет список.
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let mut st = self.conn.prepare("SELECT body FROM items WHERE kind = ?1 AND deleted = 0 AND body IS NOT NULL ORDER BY id")?;
        let rows = st.query_map([kind], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for b in rows {
            if let Ok(v) = serde_json::from_str(&b?) {
                out.push(v);
            }
        }
        Ok(out)
    }

    /// Сохранить снимок (обложку). Тот же снимок второй раз не пишется.
    pub fn put_blob(&mut self, mime: &str, data: &[u8]) -> Result<String> {
        check_blob(mime, data)?;
        let id = blob_id(data);
        self.conn.execute(
            "INSERT OR IGNORE INTO blobs(id, mime, size, data, added) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, mime, data.len() as i64, data, crate::now_secs()],
        )?;
        Ok(id)
    }

    pub fn blob(&self, id: &str) -> Result<Option<(String, Vec<u8>)>> {
        Ok(self.conn.query_row("SELECT mime, data FROM blobs WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    pub fn has_blob(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM blobs WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }

    /// Снимок под известным номером (из копии или с другого устройства): номер должен совпасть
    /// с отпечатком содержимого, а тип — с первыми байтами.
    pub fn put_blob_checked(&mut self, id: &str, mime: &str, data: &[u8]) -> Result<()> {
        if blob_id(data) != id {
            return Err(bad("снимок не совпадает со своим номером"));
        }
        self.put_blob(mime, data).map(|_| ())
    }

    /// Номера всех снимков в базе.
    pub fn present_blobs(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT id FROM blobs ORDER BY added, id")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Номера обложек, на которые ссылаются живые записи.
    pub fn referenced_blobs(&self) -> Result<std::collections::BTreeSet<String>> {
        let mut st = self.conn.prepare("SELECT json_extract(body, '$.cover') FROM items WHERE deleted = 0 AND body IS NOT NULL")?;
        let rows = st.query_map([], |r| r.get::<_, Option<String>>(0))?;
        let mut out = std::collections::BTreeSet::new();
        for r in rows {
            if let Some(id) = r? {
                out.insert(id);
            }
        }
        Ok(out)
    }

    /// Обложки, на которые ссылаются записи, но самих снимков нет (ещё не пришли с другого устройства).
    pub fn missing_blobs(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for id in self.referenced_blobs()? {
            if !self.has_blob(&id)? {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Сколько событий в журнале.
    pub fn event_count(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))? as u64)
    }

    /// Событие устройства под номером — чтобы сверить копию с базой.
    pub fn event_body(&self, device: &str, seq: i64) -> Result<Option<EventBody>> {
        let body: Option<String> = self.conn.query_row("SELECT body FROM events WHERE device = ?1 AND seq = ?2", params![device, seq], |r| r.get(0)).optional()?;
        Ok(match body {
            Some(b) => Some(serde_json::from_str(&b)?),
            None => None,
        })
    }

    /// Последний номер события каждого устройства — что эта база уже знает.
    pub fn versions(&self) -> Result<BTreeMap<String, i64>> {
        let mut st = self.conn.prepare("SELECT device, MAX(seq) FROM events GROUP BY device")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = BTreeMap::new();
        for r in rows {
            let (d, s) = r?;
            out.insert(d, s);
        }
        Ok(out)
    }

    /// События, которых нет у другого устройства (по его `versions`).
    pub fn events_missing(&self, known: &BTreeMap<String, i64>) -> Result<Vec<Event>> {
        let mut st = self.conn.prepare("SELECT device, seq, lamport, body FROM events ORDER BY lamport, device, seq")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (device, seq, lamport, body) = r?;
            if known.get(&device).is_some_and(|k| *k >= seq) {
                continue;
            }
            out.push(Event { device, seq, lamport, body: serde_json::from_str(&body)? });
        }
        Ok(out)
    }

    /// События одного устройства с номерами from..=to, по порядку.
    pub fn events_range(&self, device: &str, from: i64, to: i64) -> Result<Vec<Event>> {
        let mut st = self.conn.prepare("SELECT device, seq, lamport, body FROM events WHERE device = ?1 AND seq BETWEEN ?2 AND ?3 ORDER BY seq")?;
        let rows = st.query_map(params![device, from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (device, seq, lamport, body) = r?;
            out.push(Event { device, seq, lamport, body: serde_json::from_str(&body)? });
        }
        Ok(out)
    }

    /// Принять события другого устройства. Уже известные пропускаются. Возвращает число новых.
    pub fn apply_remote(&mut self, events: &[Event]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        for ev in events {
            let known: Option<i64> =
                tx.query_row("SELECT 1 FROM events WHERE device = ?1 AND seq = ?2", params![ev.device, ev.seq], |r| r.get(0)).optional()?;
            if known.is_some() {
                continue;
            }
            tx.execute(
                "INSERT INTO events(device, seq, lamport, body) VALUES(?1, ?2, ?3, ?4)",
                params![ev.device, ev.seq, ev.lamport, serde_json::to_string(&ev.body)?],
            )?;
            apply(&tx, ev)?;
            added += 1;
        }
        tx.commit()?;
        Ok(added)
    }
}

/// Применить событие к таблице записей: побеждает более позднее (часы Лэмпорта, затем устройство).
fn apply(tx: &Transaction, ev: &Event) -> Result<()> {
    let (kind, id, body, deleted) = match &ev.body {
        EventBody::Put { kind, id, data } => (kind, id, Some(data.to_string()), 0),
        EventBody::Del { kind, id } => (kind, id, None, 1),
    };
    let current: Option<(i64, String)> = tx
        .query_row("SELECT lamport, device FROM items WHERE kind = ?1 AND id = ?2", params![kind, id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((lamport, device)) = current {
        if (lamport, device.as_str()) > (ev.lamport, ev.device.as_str()) {
            return Ok(());
        }
    }
    tx.execute(
        "INSERT INTO items(kind, id, body, deleted, lamport, device) VALUES(?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, id) DO UPDATE SET body = excluded.body, deleted = excluded.deleted,
           lamport = excluded.lamport, device = excluded.device",
        params![kind, id, body, deleted, ev.lamport, ev.device],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Title, KIND};
    use crate::Date;

    const KEY: [u8; 32] = [7u8; 32];

    fn title(id: &str, name: &str) -> Title {
        let mut t = Title::new(Kind::Book, name, Date::parse("2026-11-13").unwrap());
        t.id = id.into();
        t
    }

    #[test]
    fn put_get_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("polka.db");
        let mut s = Store::open(&path, &KEY, "phone").unwrap();
        s.put(KIND, "a", &title("a", "Лавр")).unwrap();
        s.put(KIND, "b", &title("b", "Пикник на обочине")).unwrap();
        let mut b = title("b", "Пикник на обочине");
        b.rating = Some(9);
        s.put(KIND, "b", &b).unwrap();
        assert_eq!(s.get::<Title>(KIND, "b").unwrap().unwrap().rating, Some(9));
        assert_eq!(s.list::<Title>(KIND).unwrap().len(), 2);
        s.delete(KIND, "a").unwrap();
        assert!(s.get::<Title>(KIND, "a").unwrap().is_none());
        assert_eq!(s.list::<Title>(KIND).unwrap().len(), 1);
        drop(s);
        // Открывается снова тем же ключом; чужим — нет.
        assert!(Store::open(&path, &KEY, "phone").is_ok());
        assert!(matches!(Store::open(&path, &[1u8; 32], "phone"), Err(Error::WrongKey)));
    }

    #[test]
    fn file_is_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("polka.db");
        let mut s = Store::open(&path, &KEY, "phone").unwrap();
        s.put(KIND, "a", &title("a", "Секретное название")).unwrap();
        drop(s);
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.starts_with(b"SQLite format 3"));
        assert!(!String::from_utf8_lossy(&raw).contains("Секретное"));
    }

    #[test]
    fn two_devices_converge() {
        let dir = tempfile::tempdir().unwrap();
        let mut phone = Store::open(&dir.path().join("p.db"), &KEY, "phone").unwrap();
        let mut laptop = Store::open(&dir.path().join("l.db"), &KEY, "laptop").unwrap();
        phone.put(KIND, "a", &title("a", "Лавр")).unwrap();
        laptop.put(KIND, "b", &title("b", "Брат")).unwrap();
        // Обмен в обе стороны.
        let to_laptop = phone.events_missing(&laptop.versions().unwrap()).unwrap();
        assert_eq!(laptop.apply_remote(&to_laptop).unwrap(), 1);
        let to_phone = laptop.events_missing(&phone.versions().unwrap()).unwrap();
        assert_eq!(phone.apply_remote(&to_phone).unwrap(), 1);
        assert_eq!(phone.list::<Title>(KIND).unwrap(), laptop.list::<Title>(KIND).unwrap());
        // Повторный обмен ничего не добавляет.
        assert_eq!(phone.apply_remote(&to_phone).unwrap(), 0);
        // Правка на ноутбуке после обмена побеждает.
        let mut a = title("a", "Лавр");
        a.rating = Some(10);
        laptop.put(KIND, "a", &a).unwrap();
        let ev = laptop.events_missing(&phone.versions().unwrap()).unwrap();
        phone.apply_remote(&ev).unwrap();
        assert_eq!(phone.get::<Title>(KIND, "a").unwrap().unwrap().rating, Some(10));
    }

    #[test]
    fn blobs() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(&dir.path().join("b.db"), &KEY, "phone").unwrap();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        let id = s.put_blob("image/png", &png).unwrap();
        assert_eq!(id, s.put_blob("image/png", &png).unwrap());
        assert_eq!(s.blob(&id).unwrap().unwrap().1, png.to_vec());
        assert!(s.has_blob(&id).unwrap());
        assert!(s.put_blob("image/jpeg", &png).is_err(), "тип не совпадает с содержимым");
        assert!(s.put_blob("image/png", b"%PDF-1.4").is_err());
    }
}

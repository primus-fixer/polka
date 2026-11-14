//! Google Books: поиск по названию, автору и ISBN.
//!
//! Нужен ключ из настроек: без ключа Google отвечает 429 — дневная квота анонимных
//! запросов равна нулю (проверено 14.11.2026). С ключом — 1000 запросов в день бесплатно.

use super::http::{enc, status_error, Http};
use super::{edition_label, strs, year_in, Candidate, CoverOption};
use crate::error::{Error, Result};
use crate::isbn;
use crate::model::{Kind, SourceRef};
use serde_json::Value;

pub const NAME: &str = "Google Books";
pub const ID: &str = "google";

/// Ошибка Google понятными словами: без ключа он не отвечает совсем.
fn google_error(status: u16, key: Option<&str>) -> Error {
    let has_key = key.is_some_and(|k| !k.is_empty());
    match status {
        429 | 403 if !has_key => Error::Net(format!("{NAME}: без ключа не отвечает — укажите бесплатный ключ в настройках")),
        429 => Error::Net(format!("{NAME}: дневной лимит запросов по ключу исчерпан")),
        400 | 403 => Error::Net(format!("{NAME}: ключ не подходит — проверьте его в настройках")),
        _ => status_error(NAME, status),
    }
}

pub fn search(http: &dyn Http, q: &str, key: Option<&str>) -> Result<Vec<Candidate>> {
    let mut url = format!("https://www.googleapis.com/books/v1/volumes?q={}&maxResults=10&printType=books", enc(q));
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        url.push_str(&format!("&key={}", enc(k)));
    }
    let r = http.get(&url, &[("Accept", "application/json")], 4 << 20)?;
    if !r.ok() {
        return Err(google_error(r.status, key));
    }
    Ok(parse_search(&r.json()?))
}

pub fn parse_search(v: &Value) -> Vec<Candidate> {
    v["items"].as_array().map(|a| a.iter().filter_map(parse_volume).collect()).unwrap_or_default()
}

/// Адрес картинки Google: https и без «загнутого уголка».
pub fn fix_url(u: &str) -> String {
    u.replace("http://", "https://").replace("&edge=curl", "")
}

fn best_image(links: &Value) -> Option<String> {
    ["extraLarge", "large", "medium", "small", "thumbnail", "smallThumbnail"].iter().find_map(|k| links[*k].as_str()).map(fix_url)
}

fn parse_volume(item: &Value) -> Option<Candidate> {
    let id = item["id"].as_str()?;
    let vi = &item["volumeInfo"];
    let title = vi["title"].as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    let mut c = Candidate::new(Kind::Book, title, NAME);
    c.creators = strs(&vi["authors"]);
    c.publisher = vi["publisher"].as_str().map(String::from);
    c.year = vi["publishedDate"].as_str().and_then(year_in);
    c.pages = vi["pageCount"].as_u64().filter(|n| *n > 0).map(|n| n as u32);
    c.genres = strs(&vi["categories"]);
    if let Some(ids) = vi["industryIdentifiers"].as_array() {
        let pick = |kind: &str| ids.iter().find(|x| x["type"] == kind).and_then(|x| x["identifier"].as_str()).and_then(isbn::normalize);
        c.isbn = pick("ISBN_13").or_else(|| pick("ISBN_10"));
    }
    if let Some(u) = best_image(&vi["imageLinks"]) {
        c.covers.push(CoverOption { url: u, source: NAME.into(), edition: edition_label(&c.publisher, &c.year) });
    }
    c.about = vi["description"].as_str().map(|s| super::clean_text(s, 600));
    c.refs.push(SourceRef { provider: ID.into(), id: id.to_string() });
    Some(c)
}

fn is_volume_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 40 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Обложка покрупнее: в карточке тома бывают размеры больше, чем в поиске.
pub fn more_covers(http: &dyn Http, id: &str, key: Option<&str>) -> Result<Vec<CoverOption>> {
    if !is_volume_id(id) {
        return Ok(Vec::new());
    }
    let mut url = format!("https://www.googleapis.com/books/v1/volumes/{id}");
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        url.push_str(&format!("?key={}", enc(k)));
    }
    let r = http.get(&url, &[("Accept", "application/json")], 2 << 20)?;
    if !r.ok() {
        return Err(google_error(r.status, key));
    }
    let v = r.json()?;
    let Some(c) = parse_volume(&v) else { return Ok(Vec::new()) };
    Ok(c.covers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::http::fake::Fake;

    const SEARCH: &str = r#"{"items":[{"id":"abc_1","volumeInfo":{"title":"Пикник на обочине",
      "authors":["Аркадий Стругацкий","Борис Стругацкий"],"publisher":"АСТ","publishedDate":"2015-03-01","pageCount":256,
      "categories":["Fiction"],"industryIdentifiers":[{"type":"ISBN_10","identifier":"0306406152"}],
      "imageLinks":{"thumbnail":"http://books.google.com/books/content?id=abc_1&printsec=frontcover&img=1&zoom=1&edge=curl&source=gbs_api"}}}]}"#;

    #[test]
    fn parses_volume() {
        let http = Fake::default().route("https://www.googleapis.com/books/v1/volumes?", 200, SEARCH);
        let c = &search(&http, "пикник", Some("KEY")).unwrap()[0];
        assert_eq!(c.year.as_deref(), Some("2015"));
        assert_eq!(c.isbn.as_deref(), Some("9780306406157"));
        assert_eq!(c.covers[0].edition, "АСТ, 2015");
        assert!(c.covers[0].url.starts_with("https://books.google.com/"));
        assert!(!c.covers[0].url.contains("edge=curl"));
        assert!(http.seen.lock().unwrap()[0].ends_with("&key=KEY"));
    }

    #[test]
    fn bigger_cover_from_volume() {
        let vol = r#"{"id":"abc_1","volumeInfo":{"title":"X","imageLinks":{"thumbnail":"http://t","large":"http://books.google.com/large"}}}"#;
        let http = Fake::default().route("https://www.googleapis.com/books/v1/volumes/abc_1", 200, vol);
        assert_eq!(more_covers(&http, "abc_1", None).unwrap()[0].url, "https://books.google.com/large");
        assert!(more_covers(&http, "../x", None).unwrap().is_empty());
    }

    #[test]
    fn explains_missing_key() {
        // Так Google отвечал без ключа 14.11.2026: квота анонимных запросов — ноль.
        let body = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED"}}"#;
        let http = Fake::default().route("https://www.googleapis.com/books/v1/volumes?", 429, body);
        assert!(search(&http, "пикник", None).unwrap_err().to_string().contains("без ключа"));
        assert!(search(&http, "пикник", Some("KEY")).unwrap_err().to_string().contains("лимит"));
    }
}

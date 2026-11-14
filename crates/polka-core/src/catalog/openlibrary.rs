//! Open Library: поиск книг, издание по ISBN, другие обложки произведения.
//! Обложки берутся по номеру обложки (`/b/id/…`) — на такие адреса нет ограничения по частоте.

use super::http::{enc, status_error, Http};
use super::{strs, Candidate, CoverOption};
use crate::error::Result;
use crate::isbn;
use crate::model::{Kind, SourceRef};
use serde_json::Value;

pub const NAME: &str = "Open Library";
pub const ID: &str = "openlibrary";

pub fn cover_url(id: i64) -> String {
    format!("https://covers.openlibrary.org/b/id/{id}-L.jpg")
}

pub fn search(http: &dyn Http, q: &str) -> Result<Vec<Candidate>> {
    let url = format!(
        "https://openlibrary.org/search.json?q={}&limit=8&fields=key,title,author_name,first_publish_year,cover_i,isbn,publisher,number_of_pages_median,subject",
        enc(q)
    );
    let r = http.get(&url, &[("Accept", "application/json")], 4 << 20)?;
    if !r.ok() {
        return Err(status_error(NAME, r.status));
    }
    Ok(parse_search(&r.json()?))
}

pub fn parse_search(v: &Value) -> Vec<Candidate> {
    v["docs"].as_array().map(|docs| docs.iter().filter_map(parse_doc).collect()).unwrap_or_default()
}

fn parse_doc(d: &Value) -> Option<Candidate> {
    let title = d["title"].as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    let mut c = Candidate::new(Kind::Book, title, NAME);
    c.creators = strs(&d["author_name"]).into_iter().take(4).collect();
    c.year = d["first_publish_year"].as_i64().map(|y| y.to_string());
    c.isbn = strs(&d["isbn"]).iter().find_map(|s| isbn::normalize(s));
    c.pages = d["number_of_pages_median"].as_u64().map(|n| n as u32);
    if let Some(id) = d["cover_i"].as_i64().filter(|id| *id > 0) {
        c.covers.push(CoverOption { url: cover_url(id), source: NAME.into(), edition: String::new() });
    }
    if let Some(key) = d["key"].as_str().filter(|k| is_work_key(k)) {
        c.refs.push(SourceRef { provider: ID.into(), id: key.to_string() });
    }
    Some(c)
}

fn is_work_key(k: &str) -> bool {
    k.strip_prefix("/works/OL").and_then(|r| r.strip_suffix('W')).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Издание по ISBN (api/books): название, авторы, издательство и обложка именно этого издания.
pub fn by_isbn(http: &dyn Http, isbn13: &str) -> Result<Vec<Candidate>> {
    let url = format!("https://openlibrary.org/api/books?bibkeys=ISBN:{isbn13}&format=json&jscmd=data");
    let r = http.get(&url, &[("Accept", "application/json")], 2 << 20)?;
    if !r.ok() {
        return Err(status_error(NAME, r.status));
    }
    let v = r.json()?;
    let Some(b) = v.get(format!("ISBN:{isbn13}")) else { return Ok(Vec::new()) };
    let Some(title) = b["title"].as_str() else { return Ok(Vec::new()) };
    let mut c = Candidate::new(Kind::Book, title, NAME);
    c.creators = b["authors"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    c.publisher = b["publishers"].as_array().and_then(|a| a.first()).and_then(|p| p["name"].as_str()).map(String::from);
    c.year = b["publish_date"].as_str().and_then(super::year_in);
    c.pages = b["number_of_pages"].as_u64().map(|n| n as u32);
    c.isbn = Some(isbn13.to_string());
    let cover = ["large", "medium"].iter().find_map(|k| b["cover"][*k].as_str());
    if let Some(u) = cover {
        c.covers.push(CoverOption { url: u.replace("http://", "https://"), source: NAME.into(), edition: super::edition_label(&c.publisher, &c.year) });
    }
    if let Some(w) = b["works"].as_array().and_then(|a| a.first()).and_then(|w| w["key"].as_str()).filter(|k| is_work_key(k)) {
        c.refs.push(SourceRef { provider: ID.into(), id: w.to_string() });
    }
    Ok(vec![c])
}

/// Ещё обложки произведения: у работы в Open Library бывает несколько обложек разных изданий.
pub fn more_covers(http: &dyn Http, work_key: &str) -> Result<Vec<CoverOption>> {
    if !is_work_key(work_key) {
        return Ok(Vec::new());
    }
    let r = http.get(&format!("https://openlibrary.org{work_key}.json"), &[("Accept", "application/json")], 2 << 20)?;
    if !r.ok() {
        return Err(status_error(NAME, r.status));
    }
    let v = r.json()?;
    Ok(v["covers"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_i64()).filter(|id| *id > 0).take(8).map(|id| CoverOption { url: cover_url(id), source: NAME.into(), edition: String::new() }).collect())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::http::fake::Fake;

    const SEARCH: &str = r#"{"numFound":2,"docs":[
      {"key":"/works/OL1W","title":"Пикник на обочине","author_name":["Аркадий Стругацкий","Борис Стругацкий"],
       "first_publish_year":1972,"cover_i":12345,"isbn":["5170906492","9785170906492"],"number_of_pages_median":256},
      {"key":"/works/OL2W","title":"Roadside Picnic","author_name":["Arkady Strugatsky"],"first_publish_year":1977}
    ]}"#;

    #[test]
    fn parses_search() {
        let http = Fake::default().route("https://openlibrary.org/search.json", 200, SEARCH);
        let found = search(&http, "пикник на обочине").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Пикник на обочине");
        assert_eq!(found[0].creators.len(), 2);
        assert_eq!(found[0].year.as_deref(), Some("1972"));
        assert_eq!(found[0].covers[0].url, "https://covers.openlibrary.org/b/id/12345-L.jpg");
        assert_eq!(found[0].refs[0].id, "/works/OL1W");
        assert!(found[1].covers.is_empty());
        assert!(http.seen.lock().unwrap()[0].contains("q=%D0%BF"));
    }

    #[test]
    fn by_isbn_and_more_covers() {
        let isbn_body = r#"{"ISBN:9780306406157":{"title":"Пример","authors":[{"name":"Автор"}],"publishers":[{"name":"АСТ"}],
            "publish_date":"2021","number_of_pages":320,"cover":{"large":"http://covers.openlibrary.org/b/id/9-L.jpg"},
            "works":[{"key":"/works/OL9W"}]}}"#;
        let http = Fake::default()
            .route("https://openlibrary.org/api/books", 200, isbn_body)
            .route("https://openlibrary.org/works/OL9W.json", 200, r#"{"covers":[9,10,-1]}"#);
        let c = &by_isbn(&http, "9780306406157").unwrap()[0];
        assert_eq!(c.publisher.as_deref(), Some("АСТ"));
        assert_eq!(c.covers[0].url, "https://covers.openlibrary.org/b/id/9-L.jpg");
        assert_eq!(c.covers[0].edition, "АСТ, 2021");
        let more = more_covers(&http, "/works/OL9W").unwrap();
        assert_eq!(more.len(), 2);
        assert!(more_covers(&http, "/../etc").unwrap().is_empty());
    }
}

//! FantLab: русские издания с обложками (сильнее всего — фантастика).
//!
//! API FantLab в бета-версии и без опубликованной схемы ответа, поэтому разбор
//! осторожный: ищем в ответе объекты с `edition_id` и берём знакомые поля.
//! Обложка издания — `https://fantlab.ru/images/editions/big/{id}`.
//! Проверить на настоящих ответах — первым делом на этапе 0.

use super::http::{enc, status_error, Http};
use super::{clean_text, edition_label, year_in, Candidate, CoverOption};
use crate::error::Result;
use crate::isbn;
use crate::model::{Kind, SourceRef};
use serde_json::Value;

pub const NAME: &str = "FantLab";
pub const ID: &str = "fantlab";

pub fn cover_url(edition: &str) -> String {
    format!("https://fantlab.ru/images/editions/big/{edition}")
}

pub fn search(http: &dyn Http, q: &str) -> Result<Vec<Candidate>> {
    let url = format!("https://api.fantlab.ru/search-editions?q={}", enc(q));
    let r = http.get(&url, &[("Accept", "application/json")], 4 << 20)?;
    if !r.ok() {
        return Err(status_error(NAME, r.status));
    }
    Ok(parse_search(&r.json()?))
}

pub fn parse_search(v: &Value) -> Vec<Candidate> {
    let mut found = Vec::new();
    collect(v, &mut found, 0);
    found.into_iter().filter_map(parse_edition).take(10).collect()
}

fn collect<'a>(v: &'a Value, out: &mut Vec<&'a Value>, depth: usize) {
    if depth > 5 {
        return;
    }
    match v {
        Value::Array(a) => a.iter().for_each(|x| collect(x, out, depth + 1)),
        Value::Object(m) if m.contains_key("edition_id") => out.push(v),
        Value::Object(m) => m.values().for_each(|x| collect(x, out, depth + 1)),
        _ => {}
    }
}

fn field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match &v[*k] {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Имена авторов: «[autor=1]Аркадий Стругацкий[/autor], [autor=2]Борис…» → список имён.
fn people(s: &str) -> Vec<String> {
    clean_text(s, 400).split([',', ';']).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).take(4).collect()
}

fn parse_edition(e: &Value) -> Option<Candidate> {
    let id = field(e, &["edition_id"]).filter(|id| id.bytes().all(|b| b.is_ascii_digit()))?;
    let title = field(e, &["name", "edition_name", "title", "rusname"])?;
    let mut c = Candidate::new(Kind::Book, &clean_text(&title, 300), NAME);
    if let Some(a) = field(e, &["autors", "authors", "autor", "author"]) {
        c.creators = people(&a);
    }
    c.year = field(e, &["year", "edition_year"]).and_then(|y| year_in(&y));
    c.publisher = field(e, &["publisher", "publishers"]).map(|p| clean_text(&p, 120));
    c.isbn = field(e, &["isbn", "isbns"]).and_then(|s| s.split([',', ' ', ';']).find_map(isbn::normalize));
    c.pages = field(e, &["pages"]).and_then(|p| p.parse().ok());
    c.covers.push(CoverOption { url: cover_url(&id), source: NAME.into(), edition: edition_label(&c.publisher, &c.year) });
    c.refs.push(SourceRef { provider: ID.into(), id });
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::http::fake::Fake;

    #[test]
    fn parses_nested_editions() {
        let body = r#"{"total":2,"matches":[
          {"edition_id":101,"name":"Пикник на обочине","autors":"[autor=1]Аркадий Стругацкий[/autor], [autor=2]Борис Стругацкий[/autor]",
           "year":2021,"publisher":"АСТ","isbn":"978-0-306-40615-7"},
          {"edition_id":"102","name":"Пикник на обочине. Град обреченный","autors":"Аркадий и Борис Стругацкие","year":"2017 г."},
          {"edition_id":"x","name":"мусор"}
        ]}"#;
        let http = Fake::default().route("https://api.fantlab.ru/search-editions", 200, body);
        let found = search(&http, "пикник").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].creators, vec!["Аркадий Стругацкий".to_string(), "Борис Стругацкий".to_string()]);
        assert_eq!(found[0].covers[0].url, "https://fantlab.ru/images/editions/big/101");
        assert_eq!(found[0].covers[0].edition, "АСТ, 2021");
        assert_eq!(found[0].isbn.as_deref(), Some("9780306406157"));
        assert_eq!(found[1].year.as_deref(), Some("2017"));
        // Ответ — сразу массив.
        let arr = r#"[{"edition_id":5,"name":"Лавр"}]"#;
        assert_eq!(parse_search(&serde_json::from_str(arr).unwrap()).len(), 1);
    }
}

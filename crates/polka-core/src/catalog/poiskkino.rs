//! ПоискКино (данные Кинопоиска, TMDb и IMDb): фильмы и сериалы, постеры, рейтинги, сезоны.
//! Нужен ключ из Telegram-бота @poiskkinodev_bot; бесплатно — 200 запросов в сутки.
//! Ключ передаётся в заголовке X-API-KEY и хранится только на устройстве.

use super::http::{enc, status_error, Http};
use super::{clean_text, Candidate, CoverOption};
use crate::error::{Error, Result};
use crate::model::{Kind, SourceRef};
use serde_json::Value;

pub const NAME: &str = "ПоискКино";
pub const ID: &str = "poiskkino";
const BASE: &str = "https://api.poiskkino.dev/v1.4";

fn get(http: &dyn Http, url: &str, key: &str) -> Result<Value> {
    if key.trim().is_empty() {
        return Err(Error::Net("Для фильмов и сериалов нужен ключ ПоискКино — укажите его в настройках".into()));
    }
    let r = http.get(url, &[("X-API-KEY", key.trim()), ("Accept", "application/json")], 4 << 20)?;
    match r.status {
        200..=299 => r.json(),
        401 | 403 => Err(Error::Net("ПоискКино не принял ключ — проверьте его в настройках".into())),
        402 | 429 => Err(Error::Net("Суточный лимит запросов ПоискКино исчерпан — попробуйте завтра".into())),
        s => Err(status_error(NAME, s)),
    }
}

pub fn search(http: &dyn Http, q: &str, key: &str) -> Result<Vec<Candidate>> {
    let v = get(http, &format!("{BASE}/movie/search?page=1&limit=10&query={}", enc(q)), key)?;
    Ok(v["docs"].as_array().map(|a| a.iter().filter_map(parse_movie).collect()).unwrap_or_default())
}

/// Карточка фильма: режиссёр, сезоны и серии, длительность.
pub fn details(http: &dyn Http, id: &str, key: &str) -> Result<Candidate> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Format("неверный номер фильма".into()));
    }
    let v = get(http, &format!("{BASE}/movie/{id}"), key)?;
    parse_movie(&v).ok_or_else(|| Error::NotFound("фильм в ПоискКино".into()))
}

fn num(v: &Value) -> Option<f32> {
    v.as_f64().filter(|x| *x > 0.0).map(|x| ((x * 10.0).round() / 10.0) as f32)
}

pub fn parse_movie(m: &Value) -> Option<Candidate> {
    let id = m["id"].as_i64()?;
    let pick = |k: &str| m[k].as_str().map(str::trim).filter(|s| !s.is_empty());
    let name = pick("name").or_else(|| pick("alternativeName")).or_else(|| pick("enName"))?;
    let series = m["isSeries"].as_bool().unwrap_or(false)
        || matches!(m["type"].as_str(), Some("tv-series" | "animated-series" | "mini-series"));
    let mut c = Candidate::new(if series { Kind::Series } else { Kind::Film }, name, NAME);
    c.original = pick("alternativeName").or_else(|| pick("enName")).filter(|o| *o != name).map(String::from);
    let years = &m["releaseYears"][0];
    c.year = match (series, years["start"].as_i64(), years["end"].as_i64()) {
        (true, Some(a), Some(b)) if b > a => Some(format!("{a}–{b}")),
        (true, Some(a), None) => Some(format!("{a}–")),
        _ => m["year"].as_i64().map(|y| y.to_string()),
    };
    if let Some(u) = m["poster"]["url"].as_str().or(m["poster"]["previewUrl"].as_str()) {
        c.covers.push(CoverOption { url: u.to_string(), source: NAME.into(), edition: "Постер".into() });
    }
    c.kp = num(&m["rating"]["kp"]);
    c.imdb = num(&m["rating"]["imdb"]);
    c.minutes = m["movieLength"].as_u64().or(m["seriesLength"].as_u64()).filter(|n| *n > 0).map(|n| n as u32);
    c.about = pick("shortDescription").or_else(|| pick("description")).map(|s| clean_text(s, 600));
    c.genres = m["genres"].as_array().map(|a| a.iter().filter_map(|g| g["name"].as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(persons) = m["persons"].as_array() {
        c.creators = persons
            .iter()
            .filter(|p| p["enProfession"] == "director")
            .filter_map(|p| p["name"].as_str().or(p["enName"].as_str()).map(String::from))
            .take(3)
            .collect();
    }
    if let Some(seasons) = m["seasonsInfo"].as_array() {
        let mut s: Vec<(u32, u32)> = seasons
            .iter()
            .filter_map(|x| Some((x["number"].as_u64()? as u32, x["episodesCount"].as_u64()? as u32)))
            .filter(|(no, eps)| *no > 0 && *eps > 0)
            .collect();
        s.sort_unstable();
        s.dedup_by_key(|x| x.0);
        c.seasons = s;
    }
    c.refs.push(SourceRef { provider: ID.into(), id: id.to_string() });
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::http::fake::Fake;

    const BB: &str = r#"{"id":404900,"name":"Во все тяжкие","alternativeName":"Breaking Bad","type":"tv-series","isSeries":true,
      "year":2008,"releaseYears":[{"start":2008,"end":2013}],"poster":{"url":"https://image.example/bb.jpg"},
      "rating":{"kp":8.94,"imdb":9.5},"seriesLength":47,"shortDescription":"Учитель химии становится наркобароном",
      "genres":[{"name":"драма"},{"name":"криминал"}],
      "persons":[{"name":"Винс Гиллиган","enProfession":"director"},{"name":"Брайан Крэнстон","enProfession":"actor"}],
      "seasonsInfo":[{"number":2,"episodesCount":13},{"number":1,"episodesCount":7},{"number":0,"episodesCount":4}]}"#;

    #[test]
    fn parses_series() {
        let c = parse_movie(&serde_json::from_str(BB).unwrap()).unwrap();
        assert_eq!(c.kind, Kind::Series);
        assert_eq!(c.year.as_deref(), Some("2008–2013"));
        assert_eq!(c.original.as_deref(), Some("Breaking Bad"));
        assert_eq!(c.kp, Some(8.9));
        assert_eq!(c.creators, vec!["Винс Гиллиган".to_string()]);
        assert_eq!(c.seasons, vec![(1, 7), (2, 13)]);
        assert_eq!(c.minutes, Some(47));
    }

    #[test]
    fn needs_key_and_reports_limits() {
        let http = Fake::default().route("https://api.poiskkino.dev/v1.4/movie/search", 403, "{}");
        assert!(search(&http, "брат", "").unwrap_err().to_string().contains("ключ"));
        assert!(search(&http, "брат", "bad").unwrap_err().to_string().contains("не принял"));
        let http = Fake::default().route("https://api.poiskkino.dev/v1.4/movie/search", 200, &format!(r#"{{"docs":[{BB}]}}"#));
        assert_eq!(search(&http, "во все тяжкие", "KEY").unwrap().len(), 1);
        assert!(details(&http, "../1", "KEY").is_err());
    }
}

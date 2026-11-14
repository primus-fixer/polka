//! Добавление по ссылке: страница книги или фильма (Литрес, LiveLib, Лабиринт, Читай-город,
//! Кинопоиск и другие). Сначала разметка schema.org (JSON-LD), затем Open Graph.

use super::http::{status_error, Http};
use super::{clean_text, year_in, Candidate, CoverOption};
use crate::error::{Error, Result};
use crate::isbn;
use crate::model::Kind;
use serde_json::Value;

pub const NAME: &str = "Ссылка";

pub fn from_link(http: &dyn Http, url: &str) -> Result<Candidate> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2000 || url.contains(char::is_whitespace) {
        return Err(Error::Format("Нужна ссылка вида https://…".into()));
    }
    let r = http.get(url, &[("Accept", "text/html,application/xhtml+xml"), ("Accept-Language", "ru,en;q=0.8")], 4 << 20)?;
    if !r.ok() {
        return Err(status_error(host(url), r.status));
    }
    parse_page(&r.text(), url).ok_or_else(|| Error::NotFound("на странице не нашлось ни названия, ни обложки — добавьте вручную".into()))
}

/// Номер фильма или сериала из адреса Кинопоиска: kinopoisk.ru/film/41519/, /series/404900/.
/// Страницы Кинопоиска программе не отдаются, поэтому данные берутся из ПоискКино по этому номеру.
pub fn kinopoisk_id(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
    if host != "kinopoisk.ru" && !host.ends_with(".kinopoisk.ru") {
        return None;
    }
    let mut parts = path.split(['?', '#']).next()?.split('/').filter(|p| !p.is_empty());
    while let Some(p) = parts.next() {
        if p == "film" || p == "series" {
            let id = parts.next()?;
            return (!id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string());
        }
    }
    None
}

/// Первая ссылка в тексте из «Поделиться»: «Посмотрите «Брат» на Кинопоиске https://…».
pub fn link_in(text: &str) -> Option<&str> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let tail = &text[start..];
    let end = tail.find(|c: char| c.is_whitespace() || "<>\"«»".contains(c)).unwrap_or(tail.len());
    let url = tail[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'']);
    (url.len() > "https://".len()).then_some(url)
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("сайт")
}

/// Разобрать страницу: JSON-LD (Book, Movie, TVSeries), иначе метки Open Graph.
pub fn parse_page(html: &str, url: &str) -> Option<Candidate> {
    let source = host(url).trim_start_matches("www.").to_string();
    for block in ld_blocks(html) {
        if let Ok(v) = serde_json::from_str::<Value>(block.trim()) {
            if let Some(c) = from_ld(&v, &source, url) {
                return Some(c);
            }
        }
    }
    from_og(html, &source, url)
}

fn ld_blocks(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("application/ld+json") {
        let start = pos + i;
        let Some(gt) = lower[start..].find('>') else { break };
        let body_start = start + gt + 1;
        let Some(end) = lower[body_start..].find("</script") else { break };
        out.push(&html[body_start..body_start + end]);
        pos = body_start + end;
    }
    out
}

fn ld_objects(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().flat_map(ld_objects).collect(),
        Value::Object(m) => {
            let mut out = vec![v];
            if let Some(g) = m.get("@graph") {
                out.extend(ld_objects(g));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn types(v: &Value) -> Vec<String> {
    match &v["@type"] {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

fn names(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => s.split(',').map(|x| clean_text(x, 120)).filter(|x| !x.is_empty()).collect(),
        Value::Object(_) => v["name"].as_str().map(|s| vec![clean_text(s, 120)]).unwrap_or_default(),
        Value::Array(a) => a.iter().flat_map(names).take(4).collect(),
        _ => Vec::new(),
    }
}

fn image(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => v["url"].as_str().or(v["contentUrl"].as_str()).map(String::from),
        Value::Array(a) => a.iter().find_map(image),
        _ => None,
    }
}

fn absolute(img: &str, page: &str) -> String {
    if img.starts_with("https://") || img.starts_with("http://") {
        img.to_string()
    } else if let Some(rest) = img.strip_prefix("//") {
        format!("https://{rest}")
    } else if img.starts_with('/') {
        let scheme_host: String = page.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
        format!("{scheme_host}{img}")
    } else {
        img.to_string()
    }
}

fn from_ld(v: &Value, source: &str, url: &str) -> Option<Candidate> {
    for o in ld_objects(v) {
        let t = types(o);
        let kind = if t.iter().any(|x| x == "Book" || x == "Audiobook") {
            Kind::Book
        } else if t.iter().any(|x| x == "Movie") {
            Kind::Film
        } else if t.iter().any(|x| x == "TVSeries") {
            Kind::Series
        } else {
            continue;
        };
        let Some(name) = o["name"].as_str().map(|s| clean_text(s, 300)).filter(|s| !s.is_empty()) else { continue };
        let mut c = Candidate::new(kind, &name, source);
        c.creators = if kind == Kind::Book { names(&o["author"]) } else { names(&o["director"]) };
        c.isbn = o["isbn"].as_str().and_then(isbn::normalize);
        c.pages = o["numberOfPages"].as_u64().map(|n| n as u32).or_else(|| o["numberOfPages"].as_str().and_then(|s| s.trim().parse().ok()));
        c.year = o["datePublished"].as_str().or(o["copyrightYear"].as_str()).and_then(year_in).or_else(|| o["copyrightYear"].as_u64().map(|y| y.to_string()));
        c.publisher = names(&o["publisher"]).into_iter().next();
        c.about = o["description"].as_str().map(|s| clean_text(s, 600));
        if let Some(img) = image(&o["image"]) {
            c.covers.push(CoverOption { url: absolute(&img, url), source: source.to_string(), edition: super::edition_label(&c.publisher, &c.year) });
        }
        return Some(c);
    }
    None
}

/// Значение атрибута в открывающем теге: `content="…"` или `content='…'`.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric() && lower.as_bytes()[at - 1] != b'-';
        let rest = lower[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let off = tag.len() - rest.len() + 1;
            let val = tag[off..].trim_start();
            let q = val.chars().next()?;
            if q == '"' || q == '\'' {
                let end = val[1..].find(q)?;
                return Some(decode(&val[1..1 + end]));
            }
            let end = val.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(val.len());
            return Some(decode(&val[..end]));
        }
        from = at + name.len();
    }
    None
}

fn metas(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<meta") {
        let start = pos + i;
        let Some(end) = lower[start..].find('>') else { break };
        let tag = &html[start..start + end];
        let key = attr(tag, "property").or_else(|| attr(tag, "name"));
        if let (Some(k), Some(v)) = (key, attr(tag, "content")) {
            out.push((k.to_ascii_lowercase(), v));
        }
        pos = start + end;
    }
    out
}

fn from_og(html: &str, source: &str, url: &str) -> Option<Candidate> {
    let m = metas(html);
    let get = |k: &str| m.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).filter(|v| !v.trim().is_empty());
    let title = get("og:title")?;
    // «Название — Автор | Магазин»: имя сайта отрезаем.
    let title = clean_text(title.split(" | ").next().unwrap_or(&title), 300);
    let kind = match get("og:type").as_deref() {
        Some("video.movie") => Kind::Film,
        Some("video.tv_show") | Some("video.episode") => Kind::Series,
        _ => Kind::Book,
    };
    let mut c = Candidate::new(kind, &title, source);
    if let Some(a) = get("book:author").or_else(|| get("author")) {
        if !a.starts_with("http") {
            c.creators = a.split(',').map(|x| clean_text(x, 120)).filter(|x| !x.is_empty()).collect();
        }
    }
    c.isbn = get("book:isbn").and_then(|s| isbn::normalize(&s));
    c.year = get("book:release_date").or_else(|| get("video:release_date")).and_then(|s| year_in(&s));
    c.about = get("og:description").map(|s| clean_text(&s, 600));
    if let Some(img) = get("og:image").or_else(|| get("og:image:url")).or_else(|| get("og:image:secure_url")) {
        c.covers.push(CoverOption { url: absolute(&img, url), source: source.to_string(), edition: String::new() });
    }
    Some(c)
}

/// Сущности HTML: &amp; &quot; &#39; &laquo; &#1055; &#x41F; и т. п.
pub fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some('\u{a0}'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::http::fake::Fake;

    #[test]
    fn kinopoisk_numbers_from_addresses() {
        for (url, id) in [
            ("https://www.kinopoisk.ru/film/41519/", Some("41519")),
            ("https://kinopoisk.ru/series/404900/?utm_source=share", Some("404900")),
            ("https://m.kinopoisk.ru/film/41519", Some("41519")),
            ("http://www.kinopoisk.ru/film/41519/watch/#t", Some("41519")),
            ("https://www.kinopoisk.ru/name/7760/", None),
            ("https://hd.kinopoisk.ru/film/4f1e2d", None),
            ("https://kinopoisk.ru.evil.example/film/1/", None),
            ("https://evil.example/kinopoisk.ru/film/1/", None),
            ("https://www.kinopoisk.ru/film/../1", None),
        ] {
            assert_eq!(kinopoisk_id(url).as_deref(), id, "{url}");
        }
    }

    #[test]
    fn link_in_shared_text() {
        assert_eq!(link_in("Посмотрите «Брат» на Кинопоиске https://www.kinopoisk.ru/film/41519/"), Some("https://www.kinopoisk.ru/film/41519/"));
        assert_eq!(link_in("Пикник на обочине (https://www.litres.ru/book/piknik-6155088/)."), Some("https://www.litres.ru/book/piknik-6155088/"));
        assert_eq!(link_in("https://www.livelib.ru/book/1, советую"), Some("https://www.livelib.ru/book/1"));
        assert_eq!(link_in("без ссылки"), None);
        assert_eq!(link_in("https://"), None);
    }

    #[test]
    fn json_ld_book() {
        let html = r#"<html><head>
          <script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebPage"},
            {"@type":"Book","name":"Пикник на обочине","author":[{"@type":"Person","name":"Аркадий Стругацкий"},{"name":"Борис Стругацкий"}],
             "isbn":"978-0-306-40615-7","numberOfPages":"256","datePublished":"2021-05-01","publisher":{"name":"АСТ"},
             "image":"/covers/piknik.jpg"}]}</script></head></html>"#;
        let c = parse_page(html, "https://www.litres.ru/book/piknik/").unwrap();
        assert_eq!(c.title, "Пикник на обочине");
        assert_eq!(c.creators.len(), 2);
        assert_eq!(c.isbn.as_deref(), Some("9780306406157"));
        assert_eq!(c.pages, Some(256));
        assert_eq!(c.covers[0].url, "https://www.litres.ru/covers/piknik.jpg");
        assert_eq!(c.covers[0].source, "litres.ru");
    }

    #[test]
    fn open_graph_fallback() {
        let html = r#"<meta property="og:title" content="Лавр &mdash; Евгений Водолазкин | LiveLib">
            <meta content='https://img.example/lavr.jpg' property='og:image'>
            <meta property="book:author" content="Евгений Водолазкин">
            <meta property="og:description" content="Роман &laquo;Лавр&raquo;">"#;
        let c = parse_page(html, "https://www.livelib.ru/book/1").unwrap();
        assert_eq!(c.title, "Лавр — Евгений Водолазкин");
        assert_eq!(c.creators, vec!["Евгений Водолазкин".to_string()]);
        assert_eq!(c.covers[0].url, "https://img.example/lavr.jpg");
        assert_eq!(c.about.as_deref(), Some("Роман «Лавр»"));
    }

    #[test]
    fn link_errors() {
        let http = Fake::default().route("https://shop.example/", 200, "<html>ничего</html>");
        assert!(from_link(&http, "file:///etc/passwd").is_err());
        assert!(from_link(&http, "https://shop.example/book").unwrap_err().to_string().contains("вручную"));
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode("A &amp; B &#1055;&#x438; &unknown; &"), "A & B Пи &unknown; &");
    }
}

//! Каталоги: поиск книг, фильмов и сериалов, варианты обложек и скачивание выбранной.
//!
//! Книги ищутся сразу в трёх каталогах (Open Library, Google Books, FantLab) — параллельно.
//! Найденное складывается в произведения: одинаковые название и автор из разных каталогов
//! становятся одной строкой, а их обложки — вариантами на выбор.
//! Фильмы и сериалы — ПоискКино (нужен ключ). Любую страницу книги или фильма можно
//! добавить по ссылке.

pub mod fantlab;
pub mod google;
pub mod http;
pub mod link;
pub mod openlibrary;
pub mod poiskkino;

use crate::db::sniff_image;
use crate::error::{Error, Result};
use crate::isbn;
use crate::model::{Kind, Season, SourceRef, Title};
use crate::Date;
use http::Http;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Вариант обложки: адрес, каталог и издание («АСТ, 2021»).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverOption {
    pub url: String,
    pub source: String,
    #[serde(default)]
    pub edition: String,
}

/// Найденное произведение — ещё не запись на полке.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub original: Option<String>,
    #[serde(default)]
    pub creators: Vec<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    #[serde(default)]
    pub pages: Option<u32>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub covers: Vec<CoverOption>,
    /// В каких каталогах нашлось.
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub minutes: Option<u32>,
    #[serde(default)]
    pub kp: Option<f32>,
    #[serde(default)]
    pub imdb: Option<f32>,
    #[serde(default)]
    pub about: Option<String>,
    /// Сезоны сериала: (номер, число серий).
    #[serde(default)]
    pub seasons: Vec<(u32, u32)>,
    #[serde(default)]
    pub refs: Vec<SourceRef>,
}

impl Candidate {
    pub fn new(kind: Kind, title: &str, source: &str) -> Candidate {
        Candidate {
            kind,
            title: title.trim().to_string(),
            original: None,
            creators: Vec::new(),
            year: None,
            publisher: None,
            isbn: None,
            pages: None,
            genres: Vec::new(),
            covers: Vec::new(),
            sources: vec![source.to_string()],
            minutes: None,
            kp: None,
            imdb: None,
            about: None,
            seasons: Vec::new(),
            refs: Vec::new(),
        }
    }

    /// Запись «Хочу» из найденного; обложку приложение скачивает отдельно.
    pub fn to_title(&self, today: Date) -> Title {
        let mut t = Title::new(self.kind, &self.title, today);
        t.original = self.original.clone();
        t.creators = self.creators.clone();
        t.year = self.year.clone();
        t.publisher = self.publisher.clone();
        t.isbn = self.isbn.clone();
        t.pages = self.pages;
        t.genres = self.genres.iter().take(4).cloned().collect();
        t.minutes = self.minutes;
        t.kp = self.kp;
        t.imdb = self.imdb;
        if self.kp.is_some() || self.imdb.is_some() {
            t.ratings_on = Some(today);
        }
        t.about = self.about.clone();
        if self.kind == Kind::Series {
            t.seasons = self.seasons.iter().map(|(no, eps)| Season { no: *no, episodes: *eps, seen: Vec::new() }).collect();
        }
        t.sources = self.refs.clone();
        t
    }

    fn absorb(&mut self, other: Candidate) {
        for s in other.sources {
            if !self.sources.contains(&s) {
                self.sources.push(s);
            }
        }
        for c in other.covers {
            if !self.covers.iter().any(|x| x.url == c.url) {
                self.covers.push(c);
            }
        }
        for r in other.refs {
            if !self.refs.contains(&r) {
                self.refs.push(r);
            }
        }
        if self.creators.is_empty() {
            self.creators = other.creators;
        }
        self.original = self.original.take().or(other.original);
        self.isbn = self.isbn.take().or(other.isbn);
        self.pages = self.pages.or(other.pages);
        self.publisher = self.publisher.take().or(other.publisher);
        self.about = self.about.take().or(other.about);
        if self.genres.is_empty() {
            self.genres = other.genres;
        }
        // Год первого издания — самый ранний из найденных.
        self.year = match (self.year.take(), other.year) {
            (Some(a), Some(b)) => Some(if b.len() == 4 && a.len() == 4 && b < a { b } else { a }),
            (a, b) => a.or(b),
        };
    }
}

/// Ключи каталогов с этого устройства.
#[derive(Clone, Debug, Default)]
pub struct Keys {
    pub google: Option<String>,
    pub poiskkino: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub source: String,
    pub message: String,
}

/// Результат поиска: найденное и каталоги, которые не ответили.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Found {
    pub results: Vec<Candidate>,
    pub failed: Vec<Failure>,
}

fn take(found: &mut Vec<Vec<Candidate>>, failed: &mut Vec<Failure>, source: &str, r: std::thread::Result<Result<Vec<Candidate>>>) {
    match r {
        Ok(Ok(list)) => found.push(list),
        Ok(Err(e)) => failed.push(Failure { source: source.into(), message: e.to_string() }),
        Err(_) => failed.push(Failure { source: source.into(), message: "ошибка при разборе ответа".into() }),
    }
}

/// Книги по названию и автору — или по ISBN, если введён номер.
pub fn search_books(http: &dyn Http, query: &str, keys: &Keys) -> Found {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Found::default();
    }
    if let Some(n) = isbn::normalize(q) {
        return lookup_isbn(http, &n, keys);
    }
    let google = keys.google.as_deref();
    let (a, b, c) = std::thread::scope(|s| {
        let a = s.spawn(|| openlibrary::search(http, q));
        let b = s.spawn(|| google::search(http, q, google));
        let c = s.spawn(|| fantlab::search(http, q));
        (a.join(), b.join(), c.join())
    });
    let mut lists = Vec::new();
    let mut failed = Vec::new();
    take(&mut lists, &mut failed, fantlab::NAME, c);
    take(&mut lists, &mut failed, google::NAME, b);
    take(&mut lists, &mut failed, openlibrary::NAME, a);
    Found { results: merge(lists), failed }
}

/// Одна книга по ISBN (со штрихкода): все каталоги сразу, обложки складываются.
pub fn lookup_isbn(http: &dyn Http, isbn13: &str, keys: &Keys) -> Found {
    let google = keys.google.as_deref();
    let q = format!("isbn:{isbn13}");
    let (a, b, c) = std::thread::scope(|s| {
        let a = s.spawn(|| openlibrary::by_isbn(http, isbn13));
        let b = s.spawn(|| google::search(http, &q, google));
        let c = s.spawn(|| fantlab::search(http, isbn13));
        (a.join(), b.join(), c.join())
    });
    let mut lists = Vec::new();
    let mut failed = Vec::new();
    take(&mut lists, &mut failed, fantlab::NAME, c);
    take(&mut lists, &mut failed, google::NAME, b);
    take(&mut lists, &mut failed, openlibrary::NAME, a);
    // Каталоги по ISBN могут вернуть и соседние издания — берём только совпадающие;
    // издание без ISBN — только если это то же произведение.
    let all: Vec<Candidate> = lists.into_iter().flatten().collect();
    let same: Vec<String> = all.iter().filter(|c| c.isbn.as_deref() == Some(isbn13)).map(work_key).collect();
    let mut all: Vec<Candidate> = all
        .into_iter()
        .filter(|c| match c.isbn.as_deref() {
            Some(i) => i == isbn13,
            None => same.is_empty() || same.contains(&work_key(c)),
        })
        .collect();
    let mut results = Vec::new();
    if !all.is_empty() {
        let mut first = all.remove(0);
        for c in all {
            first.absorb(c);
        }
        first.isbn = Some(isbn13.to_string());
        results.push(first);
    }
    Found { results, failed }
}

/// Фильмы и сериалы по названию (ПоискКино).
pub fn search_films(http: &dyn Http, query: &str, keys: &Keys) -> Found {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Found::default();
    }
    match poiskkino::search(http, q, keys.poiskkino.as_deref().unwrap_or("")) {
        Ok(results) => Found { results, failed: Vec::new() },
        Err(e) => Found { results: Vec::new(), failed: vec![Failure { source: poiskkino::NAME.into(), message: e.to_string() }] },
    }
}

/// Перед выбором обложки — подробности: другие обложки книги, сезоны и режиссёр у кино.
pub fn details(http: &dyn Http, c: &Candidate, keys: &Keys) -> Candidate {
    let mut out = c.clone();
    if c.kind.is_screen() {
        if let Some(r) = c.refs.iter().find(|r| r.provider == poiskkino::ID) {
            if let Ok(full) = poiskkino::details(http, &r.id, keys.poiskkino.as_deref().unwrap_or("")) {
                let covers = std::mem::take(&mut out.covers);
                out = full;
                for cov in covers {
                    if !out.covers.iter().any(|x| x.url == cov.url) {
                        out.covers.push(cov);
                    }
                }
            }
        }
        return out;
    }
    let google = keys.google.as_deref();
    let more: Vec<CoverOption> = std::thread::scope(|s| {
        let jobs: Vec<_> = c
            .refs
            .iter()
            .map(|r| {
                s.spawn(move || match r.provider.as_str() {
                    openlibrary::ID => openlibrary::more_covers(http, &r.id).unwrap_or_default(),
                    google::ID => google::more_covers(http, &r.id, google).unwrap_or_default(),
                    _ => Vec::new(),
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    });
    for cov in more {
        // Крупная обложка Google заменяет миниатюру того же тома.
        if cov.source == google::NAME {
            out.covers.retain(|x| x.source != google::NAME || x.edition != cov.edition);
        }
        if !out.covers.iter().any(|x| x.url == cov.url) {
            out.covers.push(cov);
        }
    }
    out.covers.truncate(12);
    out
}

/// Скачать выбранную обложку: только JPEG, PNG или WebP, от 1 КБ до 6 МБ.
pub fn fetch_cover(http: &dyn Http, url: &str) -> Result<(String, Vec<u8>)> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(Error::Format("неверный адрес обложки".into()));
    }
    // У Open Library вместо «нет обложки» — пустая картинка; просим честный ответ 404.
    let url = if url.contains("covers.openlibrary.org") && !url.contains('?') { format!("{url}?default=false") } else { url.to_string() };
    let r = http.get(&url, &[("Accept", "image/avif,image/webp,image/png,image/jpeg,*/*;q=0.5")], 6 << 20)?;
    if r.status == 404 {
        return Err(Error::NotFound("у этого издания нет обложки — выберите другую".into()));
    }
    if !r.ok() {
        return Err(Error::Net(format!("обложку не скачать: ответ {}", r.status)));
    }
    let mime = sniff_image(&r.body).ok_or_else(|| Error::Format("по ссылке не картинка JPEG, PNG или WebP".into()))?;
    if r.body.len() < 1024 {
        return Err(Error::NotFound("каталог отдал пустую картинку — выберите другую обложку".into()));
    }
    Ok((mime.to_string(), r.body))
}

/// Сложить списки каталогов в произведения. Списки идут по очереди (первый — самый точный),
/// строки внутри — по релевантности; сначала — найденные в нескольких каталогах.
pub fn merge(lists: Vec<Vec<Candidate>>) -> Vec<Candidate> {
    let mut groups: Vec<(String, Candidate)> = Vec::new();
    let longest = lists.iter().map(|l| l.len()).max().unwrap_or(0);
    let mut lists: Vec<std::vec::IntoIter<Candidate>> = lists.into_iter().map(|l| l.into_iter()).collect();
    for _ in 0..longest {
        for l in lists.iter_mut() {
            let Some(c) = l.next() else { continue };
            let key = work_key(&c);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, g)) => g.absorb(c),
                None => groups.push((key, c)),
            }
        }
    }
    let mut out: Vec<Candidate> = groups.into_iter().map(|(_, c)| c).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.sources.len().min(3) * 2 + usize::from(!c.covers.is_empty())));
    out.truncate(12);
    out
}

/// Ключ произведения: название без знаков и начало фамилии первого автора.
pub fn work_key(c: &Candidate) -> String {
    let title = norm(&c.title);
    let author = c.creators.first().map(|a| norm(a)).unwrap_or_default();
    // Фамилия — последнее слово длиннее двух букв: «Стругацкий А.» и «Аркадий Стругацкий» совпадут.
    let words: Vec<&str> = author.split(' ').collect();
    let surname = words.iter().rev().find(|w| w.chars().count() > 2).or(words.last()).copied().unwrap_or("");
    let surname: String = surname.chars().take(5).collect();
    format!("{title}|{surname}")
}

/// Строчные буквы, «ё» как «е», без знаков препинания и лишних пробелов.
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        let ch = if ch == 'ё' { 'е' } else { ch };
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Строки массива JSON.
pub(crate) fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// Первые четыре цифры подряд, похожие на год: «2015-03-01», «2017 г.», «March 1977».
pub(crate) fn year_in(s: &str) -> Option<String> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let w = &b[i..i + 4];
        let digits = w.iter().all(|c| c.is_ascii_digit());
        let edge = (i == 0 || !b[i - 1].is_ascii_digit()) && (i + 4 == b.len() || !b[i + 4].is_ascii_digit());
        let y: u32 = std::str::from_utf8(w).ok()?.parse().ok()?;
        (digits && edge && (1400..=2100).contains(&y)).then(|| y.to_string())
    })
}

/// «АСТ, 2021» — подпись под вариантом обложки.
pub(crate) fn edition_label(publisher: &Option<String>, year: &Option<String>) -> String {
    match (publisher, year) {
        (Some(p), Some(y)) => format!("{p}, {y}"),
        (Some(p), None) => p.clone(),
        (None, Some(y)) => y.clone(),
        (None, None) => String::new(),
    }
}

/// Текст без тегов HTML и разметки FantLab ([autor=1]…[/autor]), без лишних пробелов, не длиннее `max` знаков.
pub(crate) fn clean_text(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut skip: Option<char> = None;
    for ch in s.chars() {
        match (skip, ch) {
            (None, '<') => skip = Some('>'),
            (None, '[') => skip = Some(']'),
            (Some(end), c) if c == end => skip = None,
            (Some(_), _) => {}
            (None, c) => out.push(if c.is_whitespace() { ' ' } else { c }),
        }
    }
    let decoded = link::decode(&out);
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max {
        let cut: String = collapsed.chars().take(max).collect();
        format!("{}…", cut.trim_end())
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::http::fake::Fake;
    use super::*;

    fn book(title: &str, author: &str, source: &str, cover: &str) -> Candidate {
        let mut c = Candidate::new(Kind::Book, title, source);
        c.creators = vec![author.into()];
        if !cover.is_empty() {
            c.covers.push(CoverOption { url: cover.into(), source: source.into(), edition: String::new() });
        }
        c
    }

    #[test]
    fn merges_same_work_from_catalogs() {
        let fl = vec![book("Пикник на обочине", "Аркадий Стругацкий", "FantLab", "https://f/1")];
        let gb = vec![
            book("Пикник на обочине.", "Аркадий и Борис Стругацкие", "Google Books", "https://g/1"),
            book("Roadside Picnic", "Arkady Strugatsky", "Google Books", ""),
        ];
        let ol = vec![book("ПИКНИК НА ОБОЧИНЕ", "Стругацкий А.", "Open Library", "https://o/1")];
        let out = merge(vec![fl, gb, ol]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "Пикник на обочине");
        assert_eq!(out[0].covers.len(), 3);
        assert_eq!(out[0].sources.len(), 3);
        assert_eq!(out[1].title, "Roadside Picnic");
    }

    #[test]
    fn helpers() {
        assert_eq!(norm("Generation «П»: роман"), "generation п роман");
        assert_eq!(norm("Ёлки"), "елки");
        assert_eq!(year_in("2017 г.").as_deref(), Some("2017"));
        assert_eq!(year_in("ISBN 9785170906492"), None);
        assert_eq!(year_in("March 1977").as_deref(), Some("1977"));
        assert_eq!(clean_text("[autor=1]Стругацкий[/autor] <b>А.</b>  Н.", 100), "Стругацкий А. Н.");
        assert_eq!(clean_text("очень длинный текст", 6), "очень…");
    }

    #[test]
    fn search_survives_failed_catalogs() {
        let http = Fake::default()
            .route("https://openlibrary.org/search.json", 200, r#"{"docs":[{"key":"/works/OL1W","title":"Лавр","author_name":["Евгений Водолазкин"],"cover_i":5}]}"#)
            .route("https://www.googleapis.com/", 503, "");
        let found = search_books(&http, "лавр водолазкин", &Keys::default());
        assert_eq!(found.results.len(), 1);
        assert_eq!(found.failed.len(), 2, "Google ответил 503, FantLab недоступен");
        assert!(found.failed.iter().any(|f| f.source == "Google Books" && f.message.contains("временно")));
    }

    #[test]
    fn isbn_query_goes_to_lookup() {
        let http = Fake::default()
            .route("https://openlibrary.org/api/books", 200, r#"{"ISBN:9780306406157":{"title":"Пример","cover":{"large":"https://covers.openlibrary.org/b/id/9-L.jpg"}}}"#)
            .route("https://www.googleapis.com/books/v1/volumes", 200, r#"{"items":[{"id":"g1","volumeInfo":{"title":"Пример","industryIdentifiers":[{"type":"ISBN_13","identifier":"9780306406157"}],"imageLinks":{"thumbnail":"http://g/1"}}}]}"#)
            .route("https://api.fantlab.ru/", 200, r#"[{"edition_id":7,"name":"Пример","isbn":"9780306406157"},{"edition_id":8,"name":"Другое","isbn":"9785170906499"},{"edition_id":9,"name":"Совсем другое"}]"#);
        let found = search_books(&http, "978-0-306-40615-7", &Keys::default());
        assert_eq!(found.results.len(), 1);
        let c = &found.results[0];
        assert_eq!(c.covers.len(), 3, "обложки трёх каталогов, без чужого издания");
        assert_eq!(c.isbn.as_deref(), Some("9780306406157"));
    }

    #[test]
    fn cover_download_checks() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.resize(2048, 0);
        let http = Fake::default()
            .route_bytes("https://img/ok.jpg", 200, jpeg)
            .route("https://img/html", 200, "<html>")
            .route("https://covers.openlibrary.org/", 404, "");
        assert_eq!(fetch_cover(&http, "https://img/ok.jpg").unwrap().0, "image/jpeg");
        assert!(fetch_cover(&http, "https://img/html").is_err());
        assert!(fetch_cover(&http, "file:///x").is_err());
        assert!(fetch_cover(&http, "https://covers.openlibrary.org/b/id/1-L.jpg").unwrap_err().to_string().contains("нет обложки"));
        assert!(http.seen.lock().unwrap().iter().any(|u| u.ends_with("?default=false")));
    }

    #[test]
    fn series_candidate_to_title() {
        let mut c = Candidate::new(Kind::Series, "Во все тяжкие", "ПоискКино");
        c.seasons = vec![(1, 7), (2, 13)];
        c.kp = Some(8.9);
        let t = c.to_title(Date::parse("2026-11-13").unwrap());
        assert_eq!(t.episodes_total(), 20);
        assert_eq!(t.ratings_on, Date::parse("2026-11-13"));
        assert!(t.check().is_ok());
    }
}

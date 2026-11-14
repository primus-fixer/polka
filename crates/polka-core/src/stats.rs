//! Итоги за год: цель по книгам, страницы и часы, по месяцам, жанры, авторы, оценки.

use crate::model::{Format, Kind, Status, Title};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Цели на год. Пустое поле — без цели.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    #[serde(default)]
    pub books: Option<u32>,
    #[serde(default)]
    pub screen: Option<u32>,
}

/// Вид записи в базе для целей; номер — год.
pub const GOAL_KIND: &str = "goal";

#[derive(Clone, Debug, Default, Serialize)]
pub struct Month {
    pub books: u32,
    /// Фильмы и сериалы, законченные в этом месяце.
    pub screen: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Count {
    pub name: String,
    pub n: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct YearStats {
    pub year: i32,
    pub goal: Goal,
    pub books: u32,
    pub films: u32,
    pub series: u32,
    /// Серий, просмотренных за год (по отметкам).
    pub episodes: u32,
    /// Страниц в прочитанных за год книгах (бумажные и электронные).
    pub pages: u32,
    /// Минут аудиокниг, прослушанных за год.
    pub audio_minutes: u32,
    /// Минут у экрана: фильмы целиком и отмеченные серии.
    pub screen_minutes: u32,
    pub dropped: u32,
    pub avg_rating: Option<f32>,
    pub months: Vec<Month>,
    pub genres: Vec<Count>,
    pub authors: Vec<Count>,
    /// Лучшая книга и лучший фильм или сериал года (по оценке).
    pub best: Vec<String>,
    /// Осталось книг до цели и дней до конца года — для подсказки «по одной за N дней».
    pub books_left: Option<u32>,
}

fn year_of(t: &Title) -> Option<i32> {
    t.finished.map(|d| d.ymd().0)
}

/// Итоги года по записям.
pub fn year_stats(titles: &[Title], year: i32, goal: Goal) -> YearStats {
    let mut s = YearStats { year, months: vec![Month::default(); 12], ..Default::default() };
    let mut ratings: Vec<u8> = Vec::new();
    let mut genres: BTreeMap<String, u32> = BTreeMap::new();
    let mut authors: BTreeMap<String, u32> = BTreeMap::new();
    let mut best_book: Option<&Title> = None;
    let mut best_screen: Option<&Title> = None;

    for t in titles {
        // Серии считаются по дням отметок, даже если сериал не досмотрен.
        if t.kind == Kind::Series {
            let eps = t.seasons.iter().flat_map(|se| se.seen.iter()).filter(|x| x.on.ymd().0 == year).count() as u32;
            s.episodes += eps;
            s.screen_minutes += eps * t.minutes.unwrap_or(0);
        }
        if year_of(t) != Some(year) {
            continue;
        }
        if t.status == Status::Dropped {
            s.dropped += 1;
            continue;
        }
        if t.status != Status::Done {
            continue;
        }
        let month = t.finished.map(|d| d.ymd().1 as usize - 1).unwrap_or(0);
        if let Some(r) = t.rating {
            ratings.push(r);
        }
        match t.kind {
            Kind::Book => {
                s.books += 1;
                s.months[month].books += 1;
                if t.format == Some(Format::Audio) {
                    s.audio_minutes += t.minutes.unwrap_or(0);
                } else {
                    s.pages += t.pages.unwrap_or(0);
                }
                for g in &t.genres {
                    *genres.entry(g.clone()).or_default() += 1;
                }
                for a in &t.creators {
                    *authors.entry(a.clone()).or_default() += 1;
                }
                if t.rating.unwrap_or(0) > best_book.and_then(|b| b.rating).unwrap_or(0) {
                    best_book = Some(t);
                }
            }
            Kind::Film | Kind::Series => {
                if t.kind == Kind::Film {
                    s.films += 1;
                    s.screen_minutes += t.minutes.unwrap_or(0);
                } else {
                    s.series += 1;
                }
                s.months[month].screen += 1;
                if t.rating.unwrap_or(0) > best_screen.and_then(|b| b.rating).unwrap_or(0) {
                    best_screen = Some(t);
                }
            }
        }
    }
    if !ratings.is_empty() {
        let avg = ratings.iter().map(|r| *r as f32).sum::<f32>() / ratings.len() as f32;
        s.avg_rating = Some((avg * 10.0).round() / 10.0);
    }
    s.genres = top(genres, 6);
    s.authors = top(authors, 5).into_iter().filter(|c| c.n > 1).collect();
    s.best = [best_book, best_screen].into_iter().flatten().map(|t| t.id.clone()).collect();
    s.books_left = goal.books.map(|g| g.saturating_sub(s.books));
    s.goal = goal;
    s
}

fn top(map: BTreeMap<String, u32>, n: usize) -> Vec<Count> {
    let mut v: Vec<Count> = map.into_iter().map(|(name, n)| Count { name, n }).collect();
    v.sort_by(|a, b| b.n.cmp(&a.n).then(a.name.cmp(&b.name)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Season;
    use crate::Date;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn book(id: &str, finished: &str, pages: u32, rating: u8, genre: &str, author: &str) -> Title {
        let mut t = Title::new(Kind::Book, id, d("2026-01-01"));
        t.id = id.into();
        t.status = Status::Done;
        t.finished = Some(d(finished));
        t.pages = Some(pages);
        t.rating = Some(rating);
        t.genres = vec![genre.into()];
        t.creators = vec![author.into()];
        t
    }

    #[test]
    fn year_totals() {
        let mut film = Title::new(Kind::Film, "Интерстеллар", d("2026-11-01"));
        film.id = "inter".into();
        film.status = Status::Done;
        film.finished = Some(d("2026-11-09"));
        film.minutes = Some(169);
        film.rating = Some(10);
        let mut series = Title::new(Kind::Series, "Во все тяжкие", d("2026-10-15"));
        series.id = "bb".into();
        series.minutes = Some(47);
        series.seasons = vec![Season { no: 1, episodes: 7, seen: vec![] }];
        series.mark_season(1, true, d("2026-10-20")).unwrap();
        let mut dropped = book("x", "2026-03-01", 300, 3, "Проза", "Кто-то");
        dropped.status = Status::Dropped;
        let old = book("old", "2025-12-30", 500, 9, "Проза", "Водолазкин");
        let titles = vec![
            book("piknik", "2026-11-12", 256, 9, "Фантастика", "Аркадий и Борис Стругацкие"),
            book("grad", "2026-06-02", 512, 8, "Фантастика", "Аркадий и Борис Стругацкие"),
            book("lavr", "2026-10-24", 440, 10, "Проза", "Евгений Водолазкин"),
            film,
            series,
            dropped,
            old,
        ];
        let s = year_stats(&titles, 2026, Goal { books: Some(24), screen: None });
        assert_eq!(s.books, 3);
        assert_eq!(s.pages, 256 + 512 + 440);
        assert_eq!(s.films, 1);
        assert_eq!(s.series, 1);
        assert_eq!(s.episodes, 7);
        assert_eq!(s.screen_minutes, 169 + 7 * 47);
        assert_eq!(s.dropped, 1);
        assert_eq!(s.months[10].books, 1);
        assert_eq!(s.months[10].screen, 1);
        assert_eq!(s.months[9].screen, 1, "сериал досмотрен в октябре");
        assert_eq!(s.genres[0].name, "Фантастика");
        assert_eq!(s.authors.len(), 1, "в авторах только те, у кого больше одной книги");
        assert_eq!(s.best, vec!["lavr".to_string(), "inter".to_string()]);
        assert_eq!(s.books_left, Some(21));
        assert_eq!(s.avg_rating, Some(9.3));
    }
}

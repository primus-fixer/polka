//! Запись на полке: книга, фильм или сериал.
//!
//! Одна запись — одно произведение. Статус — «Хочу», «В процессе», «Готово» или «Брошено».
//! У сериала — сезоны с отмеченными сериями; его статус следует за сериями сам:
//! ни одной — как был, часть — «В процессе», все — «Готово».

use crate::date::Date;
use crate::error::{bad, Error, Result};
use serde::{Deserialize, Serialize};

/// Вид записи в базе.
pub const KIND: &str = "title";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Book,
    Film,
    Series,
}

impl Kind {
    pub fn is_screen(self) -> bool {
        self != Kind::Book
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Хочу прочитать или посмотреть.
    Want,
    /// В процессе.
    Doing,
    /// Готово.
    Done,
    /// Брошено.
    Dropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Paper,
    Ebook,
    Audio,
}

/// Просмотренная серия и день просмотра.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub ep: u32,
    pub on: Date,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Season {
    pub no: u32,
    pub episodes: u32,
    #[serde(default)]
    pub seen: Vec<Seen>,
}

/// Откуда запись: каталог и номер в нём — чтобы обновить рейтинги или найти другие обложки.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub provider: String,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Title {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// Оригинальное название (у переводов и зарубежного кино).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    /// Авторы книги; режиссёр фильма; создатели сериала.
    #[serde(default)]
    pub creators: Vec<String>,
    /// «1972» или «2008–2013».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    /// Номер снимка обложки в базе (SHA-256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// Откуда обложка: «FantLab · АСТ, 2021», «своё фото».
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_source: Option<String>,
    pub status: Status,
    pub added: Date,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Date>,
    /// День, когда закончил — или бросил.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<Date>,
    /// Оценка от 1 до 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,

    // Книги.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,

    // Кино и сериалы (у аудиокниги `minutes` — длительность записи).
    /// Длительность фильма или аудиокниги; у сериала — длина одной серии.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kp: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb: Option<f32>,
    /// Когда рейтинги взяты из каталога.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratings_on: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default)]
    pub seasons: Vec<Season>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceRef>,
}

impl Title {
    /// Пустая запись «Хочу» с сегодняшней датой добавления.
    pub fn new(kind: Kind, title: &str, today: Date) -> Title {
        Title {
            id: String::new(),
            kind,
            title: title.trim().to_string(),
            original: None,
            creators: Vec::new(),
            year: None,
            cover: None,
            cover_source: None,
            status: Status::Want,
            added: today,
            started: None,
            finished: None,
            rating: None,
            note: String::new(),
            tags: Vec::new(),
            genres: Vec::new(),
            format: None,
            isbn: None,
            publisher: None,
            pages: None,
            minutes: None,
            kp: None,
            imdb: None,
            ratings_on: None,
            about: None,
            seasons: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Проверка перед записью: название, оценка, метки, даты, сезоны.
    pub fn check(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(bad("Нужно название"));
        }
        if self.title.chars().count() > 300 {
            return Err(bad("Название длиннее 300 знаков"));
        }
        if let Some(r) = self.rating {
            if !(1..=10).contains(&r) {
                return Err(bad("Оценка — от 1 до 10"));
            }
        }
        if self.note.chars().count() > 20_000 {
            return Err(bad("Заметка длиннее 20 000 знаков"));
        }
        if self.tags.len() > 30 || self.tags.iter().any(|t| t.trim().is_empty() || t.chars().count() > 40) {
            return Err(bad("Метки: не больше 30, каждая до 40 знаков"));
        }
        if let (Some(a), Some(b)) = (self.started, self.finished) {
            if b < a {
                return Err(bad("Дата окончания раньше даты начала"));
            }
        }
        if self.kind != Kind::Series && !self.seasons.is_empty() {
            return Err(bad("Сезоны бывают только у сериала"));
        }
        let mut nos: Vec<u32> = self.seasons.iter().map(|s| s.no).collect();
        nos.sort_unstable();
        nos.dedup();
        if nos.len() != self.seasons.len() {
            return Err(bad("Номера сезонов повторяются"));
        }
        for s in &self.seasons {
            if s.no == 0 || s.no > 200 || s.episodes > 2000 {
                return Err(bad(format!("Сезон {}: странное число серий", s.no)));
            }
            if s.seen.iter().any(|x| x.ep == 0 || x.ep > s.episodes) {
                return Err(bad(format!("Сезон {}: отмечена серия, которой нет", s.no)));
            }
        }
        Ok(())
    }

    /// Метки без повторов и пробелов по краям.
    pub fn tidy(&mut self) {
        self.title = self.title.trim().to_string();
        let mut tags: Vec<String> = Vec::new();
        for t in self.tags.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
            if !tags.iter().any(|x| x.to_lowercase() == t.to_lowercase()) {
                tags.push(t);
            }
        }
        self.tags = tags;
        self.creators.retain(|c| !c.trim().is_empty());
        for s in &mut self.seasons {
            s.seen.sort_by_key(|x| x.ep);
            s.seen.dedup_by_key(|x| x.ep);
        }
        self.seasons.sort_by_key(|s| s.no);
    }

    pub fn episodes_total(&self) -> u32 {
        self.seasons.iter().map(|s| s.episodes).sum()
    }

    pub fn episodes_seen(&self) -> u32 {
        self.seasons.iter().map(|s| s.seen.len() as u32).sum()
    }

    /// Следующая непросмотренная серия: (сезон, серия).
    pub fn next_episode(&self) -> Option<(u32, u32)> {
        let mut seasons: Vec<&Season> = self.seasons.iter().collect();
        seasons.sort_by_key(|s| s.no);
        for s in seasons {
            for ep in 1..=s.episodes {
                if !s.seen.iter().any(|x| x.ep == ep) {
                    return Some((s.no, ep));
                }
            }
        }
        None
    }

    /// Отметить серию просмотренной (или снять отметку).
    pub fn mark_episode(&mut self, season: u32, ep: u32, seen: bool, on: Date) -> Result<()> {
        let s = self.seasons.iter_mut().find(|s| s.no == season).ok_or_else(|| Error::NotFound(format!("сезон {season}")))?;
        if ep == 0 || ep > s.episodes {
            return Err(bad(format!("В сезоне {season} нет серии {ep}")));
        }
        s.seen.retain(|x| x.ep != ep);
        if seen {
            s.seen.push(Seen { ep, on });
            s.seen.sort_by_key(|x| x.ep);
        }
        self.follow_episodes(on);
        Ok(())
    }

    /// Отметить сезон целиком (или снять все отметки сезона).
    pub fn mark_season(&mut self, season: u32, seen: bool, on: Date) -> Result<()> {
        let s = self.seasons.iter_mut().find(|s| s.no == season).ok_or_else(|| Error::NotFound(format!("сезон {season}")))?;
        if seen {
            for ep in 1..=s.episodes {
                if !s.seen.iter().any(|x| x.ep == ep) {
                    s.seen.push(Seen { ep, on });
                }
            }
            s.seen.sort_by_key(|x| x.ep);
        } else {
            s.seen.clear();
        }
        self.follow_episodes(on);
        Ok(())
    }

    /// Статус сериала по сериям. Брошенный остаётся брошенным.
    fn follow_episodes(&mut self, today: Date) {
        if self.status == Status::Dropped {
            return;
        }
        let total = self.episodes_total();
        let seen = self.episodes_seen();
        let first = self.seasons.iter().flat_map(|s| s.seen.iter().map(|x| x.on)).min();
        let last = self.seasons.iter().flat_map(|s| s.seen.iter().map(|x| x.on)).max();
        if total > 0 && seen == total {
            self.status = Status::Done;
            if self.finished.is_none() {
                self.finished = Some(last.unwrap_or(today));
            }
            if self.started.is_none() {
                self.started = first;
            }
        } else if seen > 0 {
            self.status = Status::Doing;
            self.finished = None;
            if self.started.is_none() {
                self.started = first;
            }
        } else if self.status == Status::Done {
            self.status = if self.started.is_some() { Status::Doing } else { Status::Want };
            self.finished = None;
        }
    }

    /// Сменить статус и проставить даты: начал — сегодня, закончил или бросил — сегодня.
    pub fn set_status(&mut self, status: Status, today: Date) {
        match status {
            Status::Want => {
                self.started = None;
                self.finished = None;
            }
            Status::Doing => {
                if self.started.is_none() {
                    self.started = Some(today);
                }
                self.finished = None;
            }
            Status::Done | Status::Dropped => {
                if self.finished.is_none() || self.status != status {
                    self.finished = Some(today);
                }
                if let Some(s) = self.started {
                    if s > today {
                        self.started = Some(today);
                    }
                }
            }
        }
        self.status = status;
    }

    /// День, по которому запись стоит в ленте: закончил, начал или добавил.
    pub fn feed_date(&self) -> Date {
        self.finished.or(self.started).unwrap_or(self.added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn breaking_bad() -> Title {
        let mut t = Title::new(Kind::Series, "Во все тяжкие", d("2026-10-15"));
        t.id = "bb".into();
        t.seasons = [7, 13, 13, 13, 16].iter().enumerate().map(|(i, n)| Season { no: i as u32 + 1, episodes: *n, seen: vec![] }).collect();
        t
    }

    #[test]
    fn series_follows_episodes() {
        let mut t = breaking_bad();
        assert_eq!(t.episodes_total(), 62);
        assert_eq!(t.next_episode(), Some((1, 1)));
        t.mark_episode(1, 1, true, d("2026-10-15")).unwrap();
        assert_eq!(t.status, Status::Doing);
        assert_eq!(t.started, Some(d("2026-10-15")));
        t.mark_season(1, true, d("2026-10-20")).unwrap();
        t.mark_season(2, true, d("2026-11-01")).unwrap();
        assert_eq!(t.episodes_seen(), 20);
        assert_eq!(t.next_episode(), Some((3, 1)));
        for s in 3..=5 {
            t.mark_season(s, true, d("2026-11-13")).unwrap();
        }
        assert_eq!(t.status, Status::Done);
        assert_eq!(t.finished, Some(d("2026-11-13")));
        assert_eq!(t.next_episode(), None);
        // Сняли отметку с последней серии — снова «В процессе».
        t.mark_episode(5, 16, false, d("2026-11-14")).unwrap();
        assert_eq!(t.status, Status::Doing);
        assert_eq!(t.finished, None);
        assert!(t.mark_episode(6, 1, true, d("2026-11-14")).is_err());
        assert!(t.mark_episode(1, 8, true, d("2026-11-14")).is_err());
    }

    #[test]
    fn dropped_stays_dropped() {
        let mut t = breaking_bad();
        t.set_status(Status::Dropped, d("2026-11-01"));
        t.mark_episode(1, 1, true, d("2026-11-02")).unwrap();
        assert_eq!(t.status, Status::Dropped);
    }

    #[test]
    fn status_dates() {
        let mut t = Title::new(Kind::Book, "Пикник на обочине", d("2026-11-01"));
        t.set_status(Status::Doing, d("2026-11-02"));
        assert_eq!(t.started, Some(d("2026-11-02")));
        t.set_status(Status::Done, d("2026-11-12"));
        assert_eq!(t.finished, Some(d("2026-11-12")));
        assert_eq!(t.feed_date(), d("2026-11-12"));
        t.set_status(Status::Want, d("2026-11-13"));
        assert_eq!((t.started, t.finished), (None, None));
    }

    #[test]
    fn checks() {
        let mut t = Title::new(Kind::Book, "  ", d("2026-11-01"));
        assert!(t.check().is_err());
        t.title = "Лавр".into();
        t.rating = Some(11);
        assert!(t.check().is_err());
        t.rating = Some(10);
        t.tags = vec!["с семьёй".into(), " С семьёй ".into(), "".into()];
        t.tidy();
        assert_eq!(t.tags, vec!["с семьёй".to_string()]);
        assert!(t.check().is_ok());
        t.seasons = vec![Season { no: 1, episodes: 5, seen: vec![] }];
        assert!(t.check().is_err(), "у книги нет сезонов");
    }

    #[test]
    fn json_round_trip() {
        let mut t = breaking_bad();
        t.mark_episode(1, 3, true, d("2026-10-16")).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"kind\":\"series\""));
        assert!(!s.contains("\"isbn\""), "пустые поля не пишутся");
        let back: Title = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }
}

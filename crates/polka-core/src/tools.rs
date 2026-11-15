//! Инструменты «Полки» для Claude — коннектор к приложению Claude на компьютере.
//!
//! Чтение — `read`, запись — `plan`: она только готовит новые значения записей вместе с прежними
//! (для кнопки «Вернуть»), а записывает их приложение (`apply`). Даты — ГГГГ-ММ-ДД.

use crate::catalog::{self, Candidate};
use crate::date::Date;
use crate::db::{blob_id, Store};
use crate::error::Result;
use crate::files::{BookFile, FILE};
use crate::model::{Format, Kind, Season, Status, Title, KIND};
use crate::reader::{Mark, MarkKind, Reading, MARK, READING};
use crate::stats::{self, Goal, GOAL_KIND};
use serde_json::{json, Map, Value};

/// Описание инструмента — как его видит Claude.
#[derive(Clone, Debug)]
pub struct ToolInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    /// Меняет данные: работает, только если запись разрешена в настройках «Полки».
    pub write: bool,
}

/// Одна запись журнала: какой она была и какой станет (`None` — записи нет).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: String,
    pub id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// Подготовленная запись: новые значения, снимок обложки и что сказать человеку.
#[derive(Clone, Debug)]
pub struct Change {
    pub items: Vec<Item>,
    /// Обложка из каталога (тип и байты) — сохраняется до записей.
    pub cover: Option<(String, Vec<u8>)>,
    /// «Добавлено: «Щегол», Донна Тартт — «Хочу»».
    pub text: String,
    pub result: Value,
}

/// Найденное в каталогах для новой записи: произведение, скачанная обложка и откуда она.
#[derive(Clone, Debug)]
pub struct Lookup {
    pub candidate: Candidate,
    pub cover: Option<(String, Vec<u8>)>,
    /// «FantLab · АСТ, 2021»
    pub label: String,
}

pub type ToolResult<T> = std::result::Result<T, String>;

const KINDS: [&str; 3] = ["book", "film", "series"];
const STATUSES: [&str; 4] = ["want", "doing", "done", "dropped"];

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

/// Все инструменты по порядку: сначала чтение, потом запись.
pub fn list() -> Vec<ToolInfo> {
    let date = |d: &str| json!({ "type": "string", "description": d });
    let status = json!({ "type": "string", "enum": STATUSES, "description": "want — «Хочу»; doing — читаю или смотрю; done — прочитано или просмотрено; dropped — брошено" });
    let tags = |d: &str| json!({ "type": "array", "items": { "type": "string" }, "description": d });
    vec![
        ToolInfo {
            name: "summary",
            title: "Сводка",
            description: "Сводка полки: сколько книг, фильмов и сериалов в каждом статусе, итоги этого года и цель, что сейчас читается и смотрится (с прогрессом), последние законченные, метки. С этого стоит начинать.",
            input_schema: obj(json!({}), &[]),
            write: false,
        },
        ToolInfo {
            name: "shelf",
            title: "Полка",
            description: "Записи полки с отбором и поиском: вид, статус (want — список «Хочу»), текст, метка, жанр, автор или режиссёр, год окончания, оценка. По умолчанию — последние 50 по дате (закончено, начато или добавлено).",
            input_schema: obj(
                json!({
                    "kind": { "type": "string", "enum": ["all", "book", "film", "series", "screen"], "description": "screen — фильмы и сериалы вместе; по умолчанию all" },
                    "status": { "type": "string", "enum": ["all", "want", "doing", "done", "dropped"], "description": "По умолчанию all" },
                    "text": { "type": "string", "description": "Искать в названии, оригинальном названии, авторах, заметке и метках" },
                    "tag": { "type": "string", "description": "Метка" },
                    "genre": { "type": "string", "description": "Жанр" },
                    "creator": { "type": "string", "description": "Автор книги или режиссёр, создатель сериала" },
                    "year": { "type": "integer", "description": "Закончено (или брошено) в этом году" },
                    "min_rating": { "type": "integer", "minimum": 1, "maximum": 10, "description": "Оценка не ниже" },
                    "sort": { "type": "string", "enum": ["recent", "rating", "added", "title"], "description": "recent — по дате (по умолчанию), rating — по оценке, added — по дате добавления, title — по названию" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Сколько показать, по умолчанию 50" }
                }),
                &[],
            ),
            write: false,
        },
        ToolInfo {
            name: "title",
            title: "Запись",
            description: "Одна запись подробно: даты, оценка, заметка, метки, жанры, описание, серии сериала по сезонам, сколько прочитано в читалке, цитаты и закладки, файл книги.",
            input_schema: obj(json!({ "id": { "type": "string", "description": "id записи (из shelf или summary)" } }), &["id"]),
            write: false,
        },
        ToolInfo {
            name: "stats",
            title: "Итоги года",
            description: "Итоги года: книги, фильмы, сериалы и серии, страницы, часы аудиокниг, у экрана и в читалке, средняя оценка, по месяцам, частые жанры и авторы, лучшие книга и фильм, цель на год.",
            input_schema: obj(json!({ "year": { "type": "integer", "description": "Год, по умолчанию текущий" } }), &[]),
            write: false,
        },
        ToolInfo {
            name: "quotes",
            title: "Цитаты",
            description: "Цитаты из книг, сохранённые в читалке «Полки», с заметками — новые сверху. Можно по одной книге и с поиском.",
            input_schema: obj(
                json!({
                    "title": { "type": "string", "description": "id записи — только цитаты этой книги" },
                    "text": { "type": "string", "description": "Искать в тексте цитаты и заметке" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Сколько показать, по умолчанию 50" }
                }),
                &[],
            ),
            write: false,
        },
        ToolInfo {
            name: "add_title",
            title: "Добавить на полку",
            description: "Новая книга, фильм или сериал. Сначала проверьте через shelf, что записи ещё нет. «Полка» сама ищет в каталогах обложку, год, жанры, страницы и сезоны (look_up), если вы не выключите это. Без статуса — «Хочу».",
            input_schema: obj(
                json!({
                    "kind": { "type": "string", "enum": KINDS, "description": "book — книга (и аудиокнига), film — фильм, series — сериал" },
                    "title": { "type": "string", "description": "Название" },
                    "creators": { "type": "array", "items": { "type": "string" }, "description": "Авторы книги; режиссёр фильма; создатели сериала" },
                    "original": { "type": "string", "description": "Оригинальное название" },
                    "year": { "type": "string", "description": "Год выхода" },
                    "status": status,
                    "date": date("Дата статуса ГГГГ-ММ-ДД: добавлено, начато, закончено или брошено; по умолчанию сегодня"),
                    "rating": { "type": "integer", "minimum": 1, "maximum": 10, "description": "Оценка от 1 до 10 — для doing, done и dropped" },
                    "tags": tags("Метки; лучше из тех, что уже есть (см. summary)"),
                    "note": { "type": "string", "description": "Заметка: впечатления своими словами человека" },
                    "format": { "type": "string", "enum": ["paper", "ebook", "audio"], "description": "Книга: бумажная, электронная или аудио; по умолчанию бумажная" },
                    "seasons": { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 2000 }, "description": "Сериал: число серий в каждом сезоне по порядку, если каталог не знает" },
                    "look_up": { "type": "boolean", "description": "Искать в каталогах обложку и данные; по умолчанию да" },
                    "duplicate": { "type": "boolean", "description": "Добавить, даже если такое название уже на полке" }
                }),
                &["kind", "title"],
            ),
            write: true,
        },
        ToolInfo {
            name: "update_title",
            title: "Изменить запись",
            description: "Изменить запись: статус (даты ставятся сами, можно указать date), оценку, заметку, метки, формат книги, исправить название, авторов или год. Передавайте только то, что меняется.",
            input_schema: obj(
                json!({
                    "id": { "type": "string", "description": "id записи" },
                    "status": status,
                    "date": date("Дата нового статуса ГГГГ-ММ-ДД, по умолчанию сегодня"),
                    "started": date("Начато ГГГГ-ММ-ДД; пустая строка — убрать"),
                    "finished": date("Закончено или брошено ГГГГ-ММ-ДД; пустая строка — убрать"),
                    "rating": { "type": "integer", "minimum": 0, "maximum": 10, "description": "Оценка 1–10; 0 — убрать оценку" },
                    "note": { "type": "string", "description": "Заметка целиком (заменяет прежнюю); пустая строка — убрать" },
                    "tags": tags("Метки целиком (заменяют прежние)"),
                    "add_tags": tags("Добавить метки"),
                    "remove_tags": tags("Убрать метки"),
                    "format": { "type": "string", "enum": ["paper", "ebook", "audio"], "description": "Формат книги" },
                    "title": { "type": "string", "description": "Исправленное название" },
                    "original": { "type": "string", "description": "Оригинальное название" },
                    "creators": { "type": "array", "items": { "type": "string" }, "description": "Авторы или режиссёр целиком" },
                    "year": { "type": "string", "description": "Год выхода" }
                }),
                &["id"],
            ),
            write: true,
        },
        ToolInfo {
            name: "mark_episodes",
            title: "Серии сериала",
            description: "Отметить серии сериала просмотренными (или снять отметку): списком, с первой по N-ю или весь сезон. Статус сериала следует за сериями сам.",
            input_schema: obj(
                json!({
                    "id": { "type": "string", "description": "id сериала" },
                    "season": { "type": "integer", "minimum": 1, "description": "Номер сезона" },
                    "episodes": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "description": "Номера серий" },
                    "through": { "type": "integer", "minimum": 1, "description": "Все серии с первой по эту" },
                    "all": { "type": "boolean", "description": "Весь сезон" },
                    "seen": { "type": "boolean", "description": "true — просмотрено (по умолчанию), false — снять отметку" },
                    "date": date("День просмотра ГГГГ-ММ-ДД, по умолчанию сегодня")
                }),
                &["id", "season"],
            ),
            write: true,
        },
        ToolInfo {
            name: "delete_title",
            title: "Удалить запись",
            description: "Удалить запись с полки. Цитаты, закладки и файл книги остаются. Удаляйте только по явной просьбе человека.",
            input_schema: obj(json!({ "id": { "type": "string", "description": "id записи" } }), &["id"]),
            write: true,
        },
        ToolInfo {
            name: "set_goal",
            title: "Цель на год",
            description: "Цель на год: сколько книг прочитать и сколько фильмов и сериалов посмотреть. 0 — убрать цель; не указано — оставить как есть.",
            input_schema: obj(
                json!({
                    "year": { "type": "integer", "description": "Год, по умолчанию текущий" },
                    "books": { "type": "integer", "minimum": 0, "maximum": 1000, "description": "Книг за год" },
                    "screen": { "type": "integer", "minimum": 0, "maximum": 1000, "description": "Фильмов и сериалов за год" }
                }),
                &[],
            ),
            write: true,
        },
    ]
}

pub fn find(name: &str) -> Option<ToolInfo> {
    list().into_iter().find(|t| t.name == name)
}

// ——— Аргументы ———

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn need_str(args: &Value, key: &str) -> ToolResult<String> {
    arg_str(args, key).ok_or_else(|| format!("нужен параметр {key}"))
}

fn arg_int(args: &Value, key: &str) -> ToolResult<Option<i64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)).map(Some).ok_or_else(|| format!("{key} — целое число")),
    }
}

fn arg_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn arg_strs(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

/// Дата не позже сегодняшней.
fn parse_day(s: &str, key: &str, today: Date) -> ToolResult<Date> {
    let d = Date::parse(s).ok_or_else(|| format!("{key}: дата ГГГГ-ММ-ДД, а не «{s}»"))?;
    if d > today {
        return Err(format!("{key}: дата {d} ещё не наступила"));
    }
    if d.ymd().0 < 1900 {
        return Err(format!("{key}: слишком давняя дата"));
    }
    Ok(d)
}

fn arg_date(args: &Value, key: &str, today: Date) -> ToolResult<Option<Date>> {
    arg_str(args, key).map(|s| parse_day(&s, key, today)).transpose()
}

fn kind_of(s: &str) -> ToolResult<Kind> {
    match s {
        "book" => Ok(Kind::Book),
        "film" => Ok(Kind::Film),
        "series" => Ok(Kind::Series),
        _ => Err(format!("kind — book, film или series, а не «{s}»")),
    }
}

fn status_of(s: &str) -> ToolResult<Status> {
    match s {
        "want" => Ok(Status::Want),
        "doing" => Ok(Status::Doing),
        "done" => Ok(Status::Done),
        "dropped" => Ok(Status::Dropped),
        _ => Err(format!("status — want, doing, done или dropped, а не «{s}»")),
    }
}

fn format_of(s: &str) -> ToolResult<Format> {
    match s {
        "paper" => Ok(Format::Paper),
        "ebook" => Ok(Format::Ebook),
        "audio" => Ok(Format::Audio),
        _ => Err(format!("format — paper, ebook или audio, а не «{s}»")),
    }
}

fn code<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn status_ru(kind: Kind, s: Status) -> &'static str {
    match (s, kind) {
        (Status::Want, _) => "«Хочу»",
        (Status::Doing, Kind::Book) => "«Читаю»",
        (Status::Doing, _) => "«Смотрю»",
        (Status::Done, Kind::Book) => "«Прочитано»",
        (Status::Done, _) => "«Просмотрено»",
        (Status::Dropped, _) => "«Брошено»",
    }
}

/// «Пикник на обочине» (Аркадий Стругацкий, Борис Стругацкий)
fn named(t: &Title) -> String {
    if t.creators.is_empty() {
        format!("«{}»", t.title)
    } else {
        format!("«{}» ({})", t.title, t.creators.join(", "))
    }
}

/// Для сравнения: строчные буквы, «ё» как «е», без знаков.
fn norm(s: &str) -> String {
    catalog::norm(s)
}

/// Общая фамилия: то же слово от трёх букв или одно начало от пяти —
/// «Стругацкие» и «Аркадий Стругацкий», «Тартт Донна» и «Донна Тартт».
fn same_people(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    let words = |list: &[String]| -> Vec<String> { list.iter().flat_map(|n| norm(n).split(' ').map(str::to_string).collect::<Vec<_>>()).filter(|w| w.chars().count() >= 3).collect() };
    let stem = |w: &str| -> Option<String> { (w.chars().count() >= 5).then(|| w.chars().take(5).collect()) };
    let wb = words(b);
    words(a).iter().any(|w| wb.iter().any(|x| x == w || (stem(w).is_some() && stem(w) == stem(x))))
}

fn same_family(a: Kind, b: Kind) -> bool {
    a.is_screen() == b.is_screen()
}

// ——— Чтение ———

struct Shelf {
    titles: Vec<Title>,
    readings: Vec<Reading>,
}

impl Shelf {
    fn load(store: &Store) -> ToolResult<Shelf> {
        Ok(Shelf { titles: store.list::<Title>(KIND).map_err(|e| e.to_string())?, readings: store.list::<Reading>(READING).unwrap_or_default() })
    }

    fn reading(&self, id: &str) -> Option<&Reading> {
        self.readings.iter().find(|r| r.title == id)
    }

    fn get(&self, id: &str) -> ToolResult<&Title> {
        self.titles.iter().find(|t| t.id == id).ok_or_else(|| format!("записи «{id}» нет на полке — найдите её через shelf"))
    }
}

/// Запись коротко — для списков.
fn row(t: &Title, reading: Option<&Reading>) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(t.id));
    m.insert("kind".into(), json!(t.kind));
    m.insert("title".into(), json!(t.title));
    if let Some(o) = &t.original {
        m.insert("original".into(), json!(o));
    }
    if !t.creators.is_empty() {
        m.insert("creators".into(), json!(t.creators));
    }
    if let Some(y) = &t.year {
        m.insert("year".into(), json!(y));
    }
    m.insert("status".into(), json!(t.status));
    if let Some(r) = t.rating {
        m.insert("rating".into(), json!(r));
    }
    m.insert("added".into(), json!(t.added));
    if let Some(d) = t.started {
        m.insert("started".into(), json!(d));
    }
    if let Some(d) = t.finished {
        m.insert("finished".into(), json!(d));
    }
    if !t.tags.is_empty() {
        m.insert("tags".into(), json!(t.tags));
    }
    if let Some(f) = t.format {
        m.insert("format".into(), json!(f));
    }
    if t.kind == Kind::Series && !t.seasons.is_empty() {
        m.insert("episodes".into(), json!({ "seen": t.episodes_seen(), "total": t.episodes_total() }));
        if t.status != Status::Done {
            if let Some((s, e)) = t.next_episode() {
                m.insert("next_episode".into(), json!({ "season": s, "episode": e }));
            }
        }
    }
    if let Some(r) = reading.filter(|r| r.percent > 0.0) {
        m.insert("read_percent".into(), json!((r.percent * 10.0).round() / 10.0));
    }
    Value::Object(m)
}

fn days_desc(a: &Title, b: &Title) -> std::cmp::Ordering {
    b.feed_date().cmp(&a.feed_date()).then_with(|| b.added.cmp(&a.added)).then_with(|| a.title.cmp(&b.title))
}

pub fn read(store: &Store, today: Date, name: &str, args: &Value) -> ToolResult<Value> {
    let shelf = Shelf::load(store)?;
    match name {
        "summary" => Ok(summary(store, &shelf, today)),
        "shelf" => list_shelf(&shelf, args),
        "title" => title(store, &shelf, args),
        "stats" => year_stats(store, &shelf, today, args),
        "quotes" => quotes(store, &shelf, args),
        _ => Err(format!("«{name}» — не инструмент чтения")),
    }
}

fn goal_of(store: &Store, year: i32) -> Goal {
    store.get::<Goal>(GOAL_KIND, &year.to_string()).ok().flatten().unwrap_or_default()
}

fn reading_minutes(shelf: &Shelf, year: i32) -> u32 {
    let y = year.to_string();
    shelf.readings.iter().map(|r| r.years.get(&y).copied().unwrap_or(0)).sum()
}

fn count_tags(titles: &[Title]) -> Vec<Value> {
    let mut m: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for t in titles {
        for tag in &t.tags {
            *m.entry(tag.clone()).or_default() += 1;
        }
    }
    let mut v: Vec<(String, u32)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().take(40).map(|(name, n)| json!({ "name": name, "n": n })).collect()
}

fn summary(store: &Store, shelf: &Shelf, today: Date) -> Value {
    let year = today.ymd().0;
    let mut counts = Map::new();
    for (key, kind) in [("books", Kind::Book), ("films", Kind::Film), ("series", Kind::Series)] {
        let n = |s: Status| shelf.titles.iter().filter(|t| t.kind == kind && t.status == s).count();
        counts.insert(key.into(), json!({ "want": n(Status::Want), "doing": n(Status::Doing), "done": n(Status::Done), "dropped": n(Status::Dropped) }));
    }
    let y = stats::year_stats(&shelf.titles, year, goal_of(store, year));
    let mut doing: Vec<&Title> = shelf.titles.iter().filter(|t| t.status == Status::Doing).collect();
    doing.sort_by(|a, b| days_desc(a, b));
    let mut done: Vec<&Title> = shelf.titles.iter().filter(|t| t.status == Status::Done && t.finished.is_some()).collect();
    done.sort_by(|a, b| days_desc(a, b));
    json!({
        "today": today,
        "total": shelf.titles.len(),
        "counts": counts,
        "this_year": {
            "year": year,
            "books": y.books,
            "films": y.films,
            "series": y.series,
            "episodes": y.episodes,
            "pages": y.pages,
            "audio_hours": hours(y.audio_minutes),
            "screen_hours": hours(y.screen_minutes),
            "reading_hours": hours(reading_minutes(shelf, year)),
            "avg_rating": y.avg_rating,
            "goal": { "books": y.goal.books, "screen": y.goal.screen },
            "books_left": y.books_left
        },
        "doing": doing.iter().take(20).map(|t| row(t, shelf.reading(&t.id))).collect::<Vec<_>>(),
        "recent": done.iter().take(10).map(|t| row(t, None)).collect::<Vec<_>>(),
        "tags": count_tags(&shelf.titles)
    })
}

fn hours(min: u32) -> Value {
    json!((min as f64 / 6.0).round() / 10.0)
}

fn list_shelf(shelf: &Shelf, args: &Value) -> ToolResult<Value> {
    let kind = arg_str(args, "kind").unwrap_or_else(|| "all".into());
    if !["all", "book", "film", "series", "screen"].contains(&kind.as_str()) {
        return Err(format!("kind — all, book, film, series или screen, а не «{kind}»"));
    }
    let status = match arg_str(args, "status").as_deref() {
        None | Some("all") => None,
        Some(s) => Some(status_of(s)?),
    };
    let text = arg_str(args, "text").map(|s| norm(&s)).filter(|s| !s.is_empty());
    let tag = arg_str(args, "tag").map(|s| s.to_lowercase());
    let genre = arg_str(args, "genre").map(|s| norm(&s));
    let creator = arg_str(args, "creator").map(|s| norm(&s));
    let year = arg_int(args, "year")?;
    let min_rating = arg_int(args, "min_rating")?;
    let limit = arg_int(args, "limit")?.unwrap_or(50).clamp(1, 500) as usize;
    let mut found: Vec<&Title> = shelf
        .titles
        .iter()
        .filter(|t| match kind.as_str() {
            "book" => t.kind == Kind::Book,
            "film" => t.kind == Kind::Film,
            "series" => t.kind == Kind::Series,
            "screen" => t.kind.is_screen(),
            _ => true,
        })
        .filter(|t| status.is_none_or(|s| t.status == s))
        .filter(|t| {
            text.as_ref().is_none_or(|q| {
                let hay = norm(&[t.title.as_str(), t.original.as_deref().unwrap_or(""), &t.creators.join(" "), &t.note, &t.tags.join(" ")].join(" "));
                q.split(' ').all(|w| hay.contains(w))
            })
        })
        .filter(|t| tag.as_ref().is_none_or(|g| t.tags.iter().any(|x| x.to_lowercase() == *g)))
        .filter(|t| genre.as_ref().is_none_or(|g| t.genres.iter().any(|x| norm(x).contains(g.as_str()))))
        .filter(|t| creator.as_ref().is_none_or(|c| norm(&t.creators.join(" ")).contains(c.as_str())))
        .filter(|t| year.is_none_or(|y| matches!(t.status, Status::Done | Status::Dropped) && t.finished.is_some_and(|d| d.ymd().0 as i64 == y)))
        .filter(|t| min_rating.is_none_or(|r| t.rating.is_some_and(|x| x as i64 >= r)))
        .collect();
    match arg_str(args, "sort").as_deref() {
        None | Some("recent") => found.sort_by(|a, b| days_desc(a, b)),
        Some("rating") => found.sort_by(|a, b| b.rating.cmp(&a.rating).then_with(|| days_desc(a, b))),
        Some("added") => found.sort_by(|a, b| b.added.cmp(&a.added).then_with(|| a.title.cmp(&b.title))),
        Some("title") => found.sort_by_key(|t| norm(&t.title)),
        Some(s) => return Err(format!("sort — recent, rating, added или title, а не «{s}»")),
    }
    let total = found.len();
    Ok(json!({
        "total": total,
        "shown": total.min(limit),
        "titles": found.iter().take(limit).map(|t| row(t, shelf.reading(&t.id))).collect::<Vec<_>>()
    }))
}

fn title(store: &Store, shelf: &Shelf, args: &Value) -> ToolResult<Value> {
    let t = shelf.get(&need_str(args, "id")?)?;
    let mut v = row(t, shelf.reading(&t.id));
    let m = v.as_object_mut().expect("объект");
    if !t.note.is_empty() {
        m.insert("note".into(), json!(t.note));
    }
    if !t.genres.is_empty() {
        m.insert("genres".into(), json!(t.genres));
    }
    for (key, val) in [("publisher", &t.publisher), ("isbn", &t.isbn), ("about", &t.about)] {
        if let Some(x) = val {
            m.insert(key.into(), json!(x));
        }
    }
    if let Some(p) = t.pages {
        m.insert("pages".into(), json!(p));
    }
    if let Some(x) = t.minutes {
        m.insert(if t.kind == Kind::Series { "episode_minutes" } else { "minutes" }.into(), json!(x));
    }
    if let Some(x) = t.kp {
        m.insert("kinopoisk".into(), json!((x * 10.0).round() / 10.0));
    }
    if let Some(x) = t.imdb {
        m.insert("imdb".into(), json!((x * 10.0).round() / 10.0));
    }
    if t.kind == Kind::Series && !t.seasons.is_empty() {
        let seasons: Vec<Value> = t.seasons.iter().map(|s| json!({ "season": s.no, "episodes": s.episodes, "seen": s.seen.iter().map(|x| x.ep).collect::<Vec<_>>() })).collect();
        m.insert("seasons".into(), json!(seasons));
    }
    m.insert("cover".into(), json!(t.cover.is_some()));
    if let Some(r) = shelf.reading(&t.id) {
        let at = Date::from_epoch_days(r.at.div_euclid(86_400) as i32);
        m.insert("reading".into(), json!({ "percent": (r.percent * 10.0).round() / 10.0, "hours": hours(r.minutes), "last": at }));
    }
    let marks: Vec<Mark> = store.list::<Mark>(MARK).unwrap_or_default().into_iter().filter(|x| x.title == t.id).collect();
    let quotes = marks.iter().filter(|x| x.kind == MarkKind::Quote).count();
    if !marks.is_empty() {
        m.insert("quotes".into(), json!(quotes));
        m.insert("bookmarks".into(), json!(marks.len() - quotes));
    }
    if let Some(f) = store.list::<BookFile>(FILE).unwrap_or_default().into_iter().find(|f| f.title == t.id) {
        m.insert("book_file".into(), json!({ "name": f.name, "format": f.format }));
    }
    Ok(v)
}

fn year_stats(store: &Store, shelf: &Shelf, today: Date, args: &Value) -> ToolResult<Value> {
    let year = arg_int(args, "year")?.unwrap_or(today.ymd().0 as i64);
    if !(1900..=2200).contains(&year) {
        return Err("неверный год".into());
    }
    let year = year as i32;
    let s = stats::year_stats(&shelf.titles, year, goal_of(store, year));
    let name = |id: &String| shelf.titles.iter().find(|t| &t.id == id).map(|t| json!({ "id": t.id, "kind": t.kind, "title": t.title, "rating": t.rating }));
    let months: Vec<Value> = s.months.iter().enumerate().map(|(i, m)| json!({ "month": i + 1, "books": m.books, "screen": m.screen })).collect();
    Ok(json!({
        "year": year,
        "books": s.books,
        "films": s.films,
        "series": s.series,
        "episodes": s.episodes,
        "dropped": s.dropped,
        "pages": s.pages,
        "audio_hours": hours(s.audio_minutes),
        "screen_hours": hours(s.screen_minutes),
        "reading_hours": hours(reading_minutes(shelf, year)),
        "avg_rating": s.avg_rating,
        "months": months,
        "genres": s.genres.iter().map(|c| json!({ "name": c.name, "n": c.n })).collect::<Vec<_>>(),
        "authors": s.authors.iter().map(|c| json!({ "name": c.name, "n": c.n })).collect::<Vec<_>>(),
        "best": s.best.iter().filter_map(name).collect::<Vec<_>>(),
        "goal": { "books": s.goal.books, "screen": s.goal.screen },
        "books_left": s.books_left
    }))
}

fn quotes(store: &Store, shelf: &Shelf, args: &Value) -> ToolResult<Value> {
    let only = arg_str(args, "title");
    if let Some(id) = &only {
        shelf.get(id)?;
    }
    let text = arg_str(args, "text").map(|s| norm(&s)).filter(|s| !s.is_empty());
    let limit = arg_int(args, "limit")?.unwrap_or(50).clamp(1, 500) as usize;
    let mut list: Vec<Mark> = store
        .list::<Mark>(MARK)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|m| m.kind == MarkKind::Quote)
        .filter(|m| only.as_ref().is_none_or(|id| &m.title == id))
        .filter(|m| text.as_ref().is_none_or(|q| norm(&format!("{} {}", m.text, m.note)).contains(q.as_str())))
        .collect();
    list.sort_by_key(|m| std::cmp::Reverse(m.created));
    let total = list.len();
    let view: Vec<Value> = list
        .iter()
        .take(limit)
        .map(|m| {
            let mut v = json!({
                "id": m.id,
                "title_id": m.title,
                "title": shelf.titles.iter().find(|t| t.id == m.title).map(|t| t.title.clone()).unwrap_or_default(),
                "text": m.text,
                "percent": (m.percent * 10.0).round() / 10.0,
                "created": Date::from_epoch_days(m.created.div_euclid(86_400) as i32)
            });
            if !m.note.is_empty() {
                v["note"] = json!(m.note);
            }
            v
        })
        .collect();
    Ok(json!({ "total": total, "shown": view.len(), "quotes": view }))
}

// ——— Запись ———

/// Из найденного в каталоге — то же произведение: тот же вид (книга или кино), то же название
/// (или оригинальное) и, если авторы известны, хоть одна общая фамилия.
pub fn pick_candidate<'a>(found: &'a [Candidate], kind: Kind, title: &str, creators: &[String]) -> Option<&'a Candidate> {
    let want = norm(title);
    if want.is_empty() {
        return None;
    }
    let fits = |c: &&Candidate| same_family(c.kind, kind) && [Some(&c.title), c.original.as_ref()].into_iter().flatten().any(|n| norm(n) == want) && same_people(&c.creators, creators);
    // Сначала — точно того же вида (сериал среди фильмов).
    found.iter().filter(fits).find(|c| c.kind == kind).or_else(|| found.iter().find(fits))
}

/// Как подписать обложку: «FantLab · АСТ, 2021».
pub fn cover_label(source: &str, edition: &str) -> String {
    [source, edition].iter().filter(|s| !s.trim().is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

pub fn plan(store: &Store, today: Date, name: &str, args: &Value, lookup: Option<Lookup>) -> ToolResult<Change> {
    match name {
        "add_title" => add_title(store, today, args, lookup),
        "update_title" => update_title(store, today, args),
        "mark_episodes" => mark_episodes(store, today, args),
        "delete_title" => delete_title(store, args),
        "set_goal" => set_goal(store, today, args),
        _ => Err(format!("«{name}» — не инструмент записи")),
    }
}

fn value<T: serde::Serialize>(t: &T) -> ToolResult<Value> {
    serde_json::to_value(t).map_err(|e| e.to_string())
}

fn ready(mut t: Title) -> ToolResult<Title> {
    t.tidy();
    t.creators = t.creators.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    t.check().map_err(|e| e.to_string())?;
    Ok(t)
}

fn get_title(store: &Store, id: &str) -> ToolResult<Title> {
    store.get::<Title>(KIND, id).map_err(|e| e.to_string())?.ok_or_else(|| format!("записи «{id}» нет на полке — найдите её через shelf"))
}

/// Оценку ставят тому, что начато или закончено.
fn check_rating(r: i64, status: Status) -> ToolResult<Option<u8>> {
    if r == 0 {
        return Ok(None);
    }
    if !(1..=10).contains(&r) {
        return Err("оценка — от 1 до 10".into());
    }
    if status == Status::Want {
        return Err("у записи «Хочу» оценки нет — сначала смените статус".into());
    }
    Ok(Some(r as u8))
}

fn add_title(store: &Store, today: Date, args: &Value, lookup: Option<Lookup>) -> ToolResult<Change> {
    let kind = kind_of(&need_str(args, "kind")?)?;
    let name = need_str(args, "title")?;
    let creators = arg_strs(args, "creators").unwrap_or_default();
    let status = arg_str(args, "status").map(|s| status_of(&s)).transpose()?.unwrap_or(Status::Want);
    let on = arg_date(args, "date", today)?.unwrap_or(today);
    if arg_bool(args, "duplicate") != Some(true) {
        let titles = store.list::<Title>(KIND).map_err(|e| e.to_string())?;
        let want = norm(&name);
        if let Some(t) = titles.iter().find(|t| same_family(t.kind, kind) && [Some(&t.title), t.original.as_ref()].into_iter().flatten().any(|n| norm(n) == want) && same_people(&t.creators, &creators)) {
            return Err(format!(
                "{} уже на полке (id {}, статус {}). Чтобы изменить её — update_title; чтобы всё-таки добавить ещё одну — повторите с duplicate: true.",
                named(t),
                t.id,
                code(t.status)
            ));
        }
    }
    let lookup = lookup.filter(|l| same_family(l.candidate.kind, kind));
    let mut t = match &lookup {
        Some(l) => l.candidate.to_title(today),
        None => Title::new(kind, &name, today),
    };
    t.id = crate::new_id().map_err(|e| e.to_string())?;
    if lookup.is_none() {
        t.kind = kind;
    }
    if !creators.is_empty() {
        t.creators = creators;
    }
    if let Some(o) = arg_str(args, "original") {
        t.original = Some(o);
    }
    if let Some(y) = arg_str(args, "year") {
        t.year = Some(y.chars().take(20).collect());
    }
    if t.kind == Kind::Series && t.seasons.is_empty() {
        if let Some(list) = args.get("seasons").and_then(|v| v.as_array()) {
            t.seasons = list.iter().enumerate().filter_map(|(i, n)| n.as_u64().map(|n| Season { no: i as u32 + 1, episodes: n.min(2000) as u32, seen: Vec::new() })).collect();
        }
    }
    if status == Status::Want {
        t.added = on;
    } else {
        t.set_status(status, on);
    }
    if let Some(r) = arg_int(args, "rating")? {
        t.rating = check_rating(r, status)?;
    }
    if let Some(tags) = arg_strs(args, "tags") {
        t.tags = tags;
    }
    if let Some(n) = arg_str(args, "note") {
        t.note = n;
    }
    t.format = match (t.kind, arg_str(args, "format")) {
        (Kind::Book, Some(f)) => Some(format_of(&f)?),
        (Kind::Book, None) => Some(Format::Paper),
        _ => None,
    };
    let mut cover = None;
    if let Some(Lookup { cover: Some((mime, data)), label, .. }) = lookup.clone() {
        t.cover = Some(blob_id(&data));
        t.cover_source = Some(label).filter(|l| !l.is_empty());
        cover = Some((mime, data));
    }
    let t = ready(t)?;
    let found = match &lookup {
        Some(l) if cover.is_some() => format!("нашлось в каталогах ({}), обложка — {}", l.candidate.sources.join(", "), t.cover_source.clone().unwrap_or_default()),
        Some(l) => format!("нашлось в каталогах ({}), но без обложки — её можно выбрать в карточке", l.candidate.sources.join(", ")),
        None => "в каталогах не искали или не нашлось — обложку можно выбрать в карточке «Полки»".into(),
    };
    let text = format!("Добавлено: {} — {}", named(&t), status_ru(t.kind, t.status));
    Ok(Change {
        items: vec![Item { kind: KIND.into(), id: t.id.clone(), before: None, after: Some(value(&t)?) }],
        cover,
        result: json!({ "id": t.id, "title": row(&t, None), "catalog": found }),
        text,
    })
}

fn update_title(store: &Store, today: Date, args: &Value) -> ToolResult<Change> {
    let id = need_str(args, "id")?;
    let before = get_title(store, &id)?;
    let mut t = before.clone();
    let mut what: Vec<String> = Vec::new();
    if let Some(s) = arg_str(args, "status") {
        let s = status_of(&s)?;
        t.set_status(s, arg_date(args, "date", today)?.unwrap_or(today));
        what.push(format!("статус {}", status_ru(t.kind, s)));
    } else if arg_str(args, "date").is_some() {
        return Err("date — дата нового статуса; чтобы поправить даты, передайте started или finished".into());
    }
    for key in ["started", "finished"] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            let d = if v.trim().is_empty() { None } else { Some(parse_day(v.trim(), key, today)?) };
            if key == "started" {
                t.started = d;
            } else {
                t.finished = d;
            }
            what.push(if key == "started" { "дата начала".into() } else { "дата окончания".into() });
        }
    }
    if let Some(r) = arg_int(args, "rating")? {
        t.rating = check_rating(r, t.status)?;
        what.push(t.rating.map_or("без оценки".into(), |r| format!("оценка {r} из 10")));
    }
    if let Some(n) = args.get("note").and_then(|v| v.as_str()) {
        t.note = n.trim().to_string();
        what.push(if t.note.is_empty() { "без заметки".into() } else { "заметка".into() });
    }
    if let Some(tags) = arg_strs(args, "tags") {
        t.tags = tags;
        what.push("метки".into());
    }
    if let Some(add) = arg_strs(args, "add_tags") {
        t.tags.extend(add);
        what.push("метки".into());
    }
    if let Some(rm) = arg_strs(args, "remove_tags") {
        let rm: Vec<String> = rm.iter().map(|x| x.to_lowercase()).collect();
        t.tags.retain(|x| !rm.contains(&x.to_lowercase()));
        what.push("метки".into());
    }
    if let Some(f) = arg_str(args, "format") {
        if t.kind != Kind::Book {
            return Err("формат бывает только у книги".into());
        }
        t.format = Some(format_of(&f)?);
        what.push("формат".into());
    }
    if let Some(x) = arg_str(args, "title") {
        t.title = x;
        what.push("название".into());
    }
    if let Some(x) = arg_str(args, "original") {
        t.original = Some(x);
        what.push("оригинальное название".into());
    }
    if let Some(x) = arg_strs(args, "creators") {
        t.creators = x;
        what.push(if t.kind == Kind::Book { "авторы".into() } else { "создатели".into() });
    }
    if let Some(x) = arg_str(args, "year") {
        t.year = Some(x.chars().take(20).collect());
        what.push("год".into());
    }
    let t = ready(t)?;
    if t == before {
        return Err("ничего не изменилось".into());
    }
    what.dedup();
    let text = format!("Изменено: {} — {}", named(&t), what.join(", "));
    Ok(Change {
        items: vec![Item { kind: KIND.into(), id: t.id.clone(), before: Some(value(&before)?), after: Some(value(&t)?) }],
        cover: None,
        result: json!({ "title": row(&t, None) }),
        text,
    })
}

/// «1–6», «2, 4, 7».
fn eps_text(eps: &[u32]) -> String {
    let mut v = eps.to_vec();
    v.sort_unstable();
    v.dedup();
    if v.len() > 2 && v.windows(2).all(|w| w[1] == w[0] + 1) {
        return format!("{}–{}", v[0], v[v.len() - 1]);
    }
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")
}

fn mark_episodes(store: &Store, today: Date, args: &Value) -> ToolResult<Change> {
    let id = need_str(args, "id")?;
    let before = get_title(store, &id)?;
    if before.kind != Kind::Series {
        return Err(format!("{} — не сериал", named(&before)));
    }
    if before.seasons.is_empty() {
        return Err(format!("у {} не указаны сезоны — их можно добавить в карточке «Полки»", named(&before)));
    }
    let season = arg_int(args, "season")?.ok_or("нужен параметр season")?;
    let Some(s) = before.seasons.iter().find(|s| s.no as i64 == season) else {
        let nos: Vec<String> = before.seasons.iter().map(|s| s.no.to_string()).collect();
        return Err(format!("у {} нет сезона {season}; есть: {}", named(&before), nos.join(", ")));
    };
    let season = s.no;
    let seen = arg_bool(args, "seen").unwrap_or(true);
    let on = arg_date(args, "date", today)?.unwrap_or(today);
    let mut t = before.clone();
    let eps: Vec<u32> = if arg_bool(args, "all") == Some(true) {
        t.mark_season(season, seen, on).map_err(|e| e.to_string())?;
        (1..=s.episodes).collect()
    } else {
        let list: Vec<u32> = match (args.get("episodes").and_then(|v| v.as_array()), arg_int(args, "through")?) {
            (Some(a), _) => a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect(),
            (None, Some(n)) if n >= 1 => (1..=n as u32).collect(),
            _ => return Err("укажите episodes (номера серий), through (по какую серию) или all: true".into()),
        };
        if list.is_empty() {
            return Err("пустой список серий".into());
        }
        for ep in &list {
            t.mark_episode(season, *ep, seen, on).map_err(|e| e.to_string())?;
        }
        list
    };
    let t = ready(t)?;
    if t == before {
        return Err(if seen { "эти серии уже отмечены" } else { "эти серии и так не отмечены" }.into());
    }
    let text = format!("{}: {}, сезон {season}, серии {} — {}", if seen { "Отмечено" } else { "Снята отметка" }, named(&t), eps_text(&eps), status_ru(t.kind, t.status));
    Ok(Change {
        items: vec![Item { kind: KIND.into(), id: t.id.clone(), before: Some(value(&before)?), after: Some(value(&t)?) }],
        cover: None,
        result: json!({ "title": row(&t, None) }),
        text,
    })
}

fn delete_title(store: &Store, args: &Value) -> ToolResult<Change> {
    let id = need_str(args, "id")?;
    let before = get_title(store, &id)?;
    Ok(Change {
        items: vec![Item { kind: KIND.into(), id: id.clone(), before: Some(value(&before)?), after: None }],
        cover: None,
        text: format!("Удалено с полки: {}", named(&before)),
        result: json!({ "deleted": id, "note": "цитаты, закладки и файл книги остались" }),
    })
}

fn set_goal(store: &Store, today: Date, args: &Value) -> ToolResult<Change> {
    let year = arg_int(args, "year")?.unwrap_or(today.ymd().0 as i64);
    if !(1900..=2200).contains(&year) {
        return Err("неверный год".into());
    }
    let id = year.to_string();
    let before: Option<Goal> = store.get(GOAL_KIND, &id).map_err(|e| e.to_string())?;
    let mut goal = before.clone().unwrap_or_default();
    let num = |key: &str| -> ToolResult<Option<Option<u32>>> {
        Ok(match arg_int(args, key)? {
            None => None,
            Some(0) => Some(None),
            Some(n) if (1..=1000).contains(&n) => Some(Some(n as u32)),
            Some(_) => return Err(format!("{key} — от 0 до 1000")),
        })
    };
    let (books, screen) = (num("books")?, num("screen")?);
    if books.is_none() && screen.is_none() {
        return Err("укажите books или screen".into());
    }
    if let Some(b) = books {
        goal.books = b;
    }
    if let Some(s) = screen {
        goal.screen = s;
    }
    if before.as_ref() == Some(&goal) {
        return Err("цель и так такая".into());
    }
    let part = |n: Option<u32>, what: &str| n.map_or(format!("{what} — без цели"), |n| format!("{what} — {n}"));
    Ok(Change {
        items: vec![Item { kind: GOAL_KIND.into(), id, before: before.as_ref().map(value).transpose()?, after: Some(value(&goal)?) }],
        cover: None,
        text: format!("Цель на {year} год: {}, {}", part(goal.books, "книги"), part(goal.screen, "фильмы и сериалы")),
        result: json!({ "year": year, "books": goal.books, "screen": goal.screen }),
    })
}

// ——— Применение и отмена ———

/// Записать подготовленное: снимок обложки, потом записи журнала.
pub fn apply(store: &mut Store, change: &Change) -> Result<()> {
    if let Some((mime, data)) = &change.cover {
        store.put_blob(mime, data)?;
    }
    for it in &change.items {
        match &it.after {
            Some(v) => store.put(&it.kind, &it.id, v)?,
            None => store.delete(&it.kind, &it.id)?,
        }
    }
    Ok(())
}

/// Записанное Claude ещё на месте — после него никто ничего не менял (ни здесь, ни на другом устройстве).
pub fn still_current(store: &Store, items: &[Item]) -> Result<bool> {
    for it in items {
        if store.get::<Value>(&it.kind, &it.id)? != it.after {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Вернуть как было — в обратном порядке.
pub fn undo(store: &mut Store, items: &[Item]) -> Result<()> {
    for it in items.iter().rev() {
        match &it.before {
            Some(v) => store.put(&it.kind, &it.id, v)?,
            None => store.delete(&it.kind, &it.id)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CoverOption;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let st = Store::open(&dir.path().join("t.db"), &[3; 32], "dev1").unwrap();
        (dir, st)
    }

    fn today() -> Date {
        d("2026-11-15")
    }

    fn put(st: &mut Store, t: &Title) {
        st.put(KIND, &t.id, t).unwrap();
    }

    fn sample(st: &mut Store) {
        let mut a = Title::new(Kind::Book, "Пикник на обочине", d("2026-10-01"));
        a.id = "piknik".into();
        a.creators = vec!["Аркадий Стругацкий".into(), "Борис Стругацкий".into()];
        a.set_status(Status::Done, d("2026-11-12"));
        a.rating = Some(9);
        a.tags = vec!["в командировке".into()];
        a.format = Some(Format::Paper);
        a.pages = Some(256);
        put(st, &a);
        let mut b = Title::new(Kind::Series, "Во все тяжкие", d("2026-10-15"));
        b.id = "bb".into();
        b.original = Some("Breaking Bad".into());
        b.seasons = [7, 13, 13, 13, 16].iter().enumerate().map(|(i, n)| Season { no: i as u32 + 1, episodes: *n, seen: vec![] }).collect();
        put(st, &b);
        let mut c = Title::new(Kind::Film, "Оппенгеймер", d("2026-11-10"));
        c.id = "opp".into();
        put(st, &c);
    }

    fn apply_ok(st: &mut Store, name: &str, args: Value) -> Change {
        let c = plan(st, today(), name, &args, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        apply(st, &c).unwrap();
        c
    }

    #[test]
    fn every_tool_has_a_schema_and_reads_work() {
        let (_d, mut st) = store();
        sample(&mut st);
        for t in list() {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(find(t.name).is_some());
        }
        let s = read(&st, today(), "summary", &json!({})).unwrap();
        assert_eq!(s["counts"]["books"]["done"], 1);
        assert_eq!(s["counts"]["series"]["want"], 1);
        assert_eq!(s["this_year"]["books"], 1);
        assert_eq!(s["tags"][0]["name"], "в командировке");
        let list = read(&st, today(), "shelf", &json!({ "status": "want", "kind": "screen" })).unwrap();
        assert_eq!(list["total"], 2);
        let found = read(&st, today(), "shelf", &json!({ "text": "breaking" })).unwrap();
        assert_eq!(found["titles"][0]["id"], "bb");
        assert_eq!(found["titles"][0]["episodes"]["total"], 62);
        let found = read(&st, today(), "shelf", &json!({ "creator": "стругацкий", "year": 2026, "min_rating": 9 })).unwrap();
        assert_eq!(found["total"], 1);
        let one = read(&st, today(), "title", &json!({ "id": "piknik" })).unwrap();
        assert_eq!(one["pages"], 256);
        assert_eq!(one["cover"], false);
        assert!(read(&st, today(), "title", &json!({ "id": "нет" })).is_err());
        let y = read(&st, today(), "stats", &json!({ "year": 2026 })).unwrap();
        assert_eq!(y["best"][0]["title"], "Пикник на обочине");
        assert_eq!(read(&st, today(), "quotes", &json!({})).unwrap()["total"], 0);
        assert!(read(&st, today(), "shelf", &json!({ "sort": "random" })).is_err());
    }

    #[test]
    fn add_checks_duplicates_and_undo_removes_it() {
        let (_d, mut st) = store();
        sample(&mut st);
        let dup = plan(&st, today(), "add_title", &json!({ "kind": "book", "title": "пикник на обочине!", "creators": ["Стругацкие"] }), None);
        assert!(dup.unwrap_err().contains("уже на полке"));
        let c = apply_ok(&mut st, "add_title", json!({ "kind": "book", "title": "Щегол", "creators": ["Донна Тартт"], "tags": ["отпуск"] }));
        assert!(c.text.contains("Щегол") && c.text.contains("«Хочу»"), "{}", c.text);
        let id = c.result["id"].as_str().unwrap().to_string();
        let t: Title = st.get(KIND, &id).unwrap().unwrap();
        assert_eq!((t.status, t.format, t.added), (Status::Want, Some(Format::Paper), today()));
        assert!(still_current(&st, &c.items).unwrap());
        undo(&mut st, &c.items).unwrap();
        assert!(st.get::<Title>(KIND, &id).unwrap().is_none(), "«Вернуть» убрало запись");
        // Прочитано вчера, 8 из 10.
        let c = apply_ok(&mut st, "add_title", json!({ "kind": "book", "title": "Лавр", "status": "done", "date": "2026-11-14", "rating": 8 }));
        let t: Title = st.get(KIND, c.result["id"].as_str().unwrap()).unwrap().unwrap();
        assert_eq!((t.status, t.finished, t.rating), (Status::Done, Some(d("2026-11-14")), Some(8)));
        assert!(plan(&st, today(), "add_title", &json!({ "kind": "book", "title": "X", "rating": 5 }), None).unwrap_err().contains("«Хочу»"));
        assert!(plan(&st, today(), "add_title", &json!({ "kind": "book", "title": "X", "status": "done", "date": "2027-01-01" }), None).unwrap_err().contains("не наступила"));
        assert!(plan(&st, today(), "add_title", &json!({ "kind": "album", "title": "X" }), None).is_err());
    }

    #[test]
    fn add_takes_cover_and_details_from_the_catalog() {
        let (_d, mut st) = store();
        let mut cand = Candidate::new(Kind::Series, "Чернобыль", "ПоискКино");
        cand.creators = vec!["Юхан Ренк".into()];
        cand.year = Some("2019".into());
        cand.seasons = vec![(1, 5)];
        cand.covers = vec![CoverOption { url: "https://example.org/c.jpg".into(), source: "ПоискКино".into(), edition: "Постер".into() }];
        let found = vec![Candidate::new(Kind::Film, "Чернобыль: Зона отчуждения", "ПоискКино"), cand.clone()];
        let picked = pick_candidate(&found, Kind::Series, "чернобыль", &[]).unwrap();
        assert_eq!(picked.year.as_deref(), Some("2019"));
        assert!(pick_candidate(&found, Kind::Book, "Чернобыль", &[]).is_none(), "книга — не сериал");
        assert!(pick_candidate(&found, Kind::Series, "Чернобыль", &["Иван Петров".into()]).is_none(), "другой автор");
        let jpeg = b"\xFF\xD8\xFF\xE0 cover".repeat(200);
        let look = Lookup { candidate: cand, cover: Some(("image/jpeg".into(), jpeg.clone())), label: cover_label("ПоискКино", "Постер") };
        let c = plan(&st, today(), "add_title", &json!({ "kind": "series", "title": "Чернобыль", "status": "want" }), Some(look)).unwrap();
        apply(&mut st, &c).unwrap();
        let t: Title = st.get(KIND, c.result["id"].as_str().unwrap()).unwrap().unwrap();
        assert_eq!((t.episodes_total(), t.year.as_deref(), t.cover_source.as_deref()), (5, Some("2019"), Some("ПоискКино · Постер")));
        assert_eq!(t.cover.as_deref(), Some(blob_id(&jpeg).as_str()));
        assert!(st.blob(&blob_id(&jpeg)).unwrap().is_some(), "снимок обложки в базе");
        assert!(c.result["catalog"].as_str().unwrap().contains("ПоискКино"));
    }

    #[test]
    fn update_mark_delete_goal_and_undo_rules() {
        let (_d, mut st) = store();
        sample(&mut st);
        let c = apply_ok(&mut st, "update_title", json!({ "id": "opp", "status": "done", "rating": 9, "add_tags": ["с семьёй"] }));
        assert!(c.text.contains("оценка 9 из 10"), "{}", c.text);
        let t: Title = st.get(KIND, "opp").unwrap().unwrap();
        assert_eq!((t.status, t.finished, t.rating, t.tags.clone()), (Status::Done, Some(today()), Some(9), vec!["с семьёй".to_string()]));
        assert!(plan(&st, today(), "update_title", &json!({ "id": "opp", "rating": 9 }), None).unwrap_err().contains("ничего не изменилось"));
        // После Claude запись поправили в «Полке» — отменить нельзя.
        let mut edited = t.clone();
        edited.note = "Сильный фильм".into();
        put(&mut st, &edited);
        assert!(!still_current(&st, &c.items).unwrap());

        let c = apply_ok(&mut st, "mark_episodes", json!({ "id": "bb", "season": 1, "all": true }));
        assert!(c.text.contains("сезон 1, серии 1–7") && c.text.contains("«Смотрю»"), "{}", c.text);
        apply_ok(&mut st, "mark_episodes", json!({ "id": "bb", "season": 2, "through": 3 }));
        let t: Title = st.get(KIND, "bb").unwrap().unwrap();
        assert_eq!((t.episodes_seen(), t.status), (10, Status::Doing));
        assert!(plan(&st, today(), "mark_episodes", &json!({ "id": "bb", "season": 9, "all": true }), None).unwrap_err().contains("нет сезона 9"));
        assert!(plan(&st, today(), "mark_episodes", &json!({ "id": "piknik", "season": 1, "all": true }), None).unwrap_err().contains("не сериал"));

        let c = apply_ok(&mut st, "delete_title", json!({ "id": "piknik" }));
        assert!(st.get::<Title>(KIND, "piknik").unwrap().is_none());
        assert!(still_current(&st, &c.items).unwrap());
        undo(&mut st, &c.items).unwrap();
        assert_eq!(st.get::<Title>(KIND, "piknik").unwrap().unwrap().rating, Some(9), "«Вернуть» вернуло запись");

        let c = apply_ok(&mut st, "set_goal", json!({ "books": 24 }));
        assert_eq!(st.get::<Goal>(GOAL_KIND, "2026").unwrap().unwrap().books, Some(24));
        assert!(c.text.contains("книги — 24"), "{}", c.text);
        assert!(plan(&st, today(), "set_goal", &json!({ "books": 24 }), None).unwrap_err().contains("и так"));
        undo(&mut st, &c.items).unwrap();
        assert_eq!(st.get::<Goal>(GOAL_KIND, "2026").unwrap(), None, "цели не было — и снова нет");
    }

    #[test]
    fn episode_ranges_read_well() {
        assert_eq!(eps_text(&[3, 1, 2, 4]), "1–4");
        assert_eq!(eps_text(&[2, 4, 7]), "2, 4, 7");
        assert_eq!(eps_text(&[5]), "5");
    }
}

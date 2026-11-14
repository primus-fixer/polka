//! Читалка: книги EPUB и FB2.
//!
//! Книга открывается из байтов файла целиком, на диск ничего не распаковывается. Главы сразу
//! переводятся в безопасное дерево для страницы (`tree`): EPUB — из XHTML с учётом стилей книги
//! (`html`, `css`), FB2 — из его разметки (`fb2`). Картинки отдаются по пути внутри книги.
//! Место чтения (`Reading`) — запись журнала: обменивается между устройствами, как полка.

pub mod css;
pub mod epub;
pub mod fb2;
pub mod html;
pub mod markup;
pub mod shelf;
pub mod tree;

use crate::error::{bad, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::sync::Mutex;
use tree::Out;

/// Больше такой книги не открываем.
pub const MAX_BOOK: usize = 100 << 20;
/// Предел одной записи в архиве после распаковки.
const MAX_ENTRY: u64 = 64 << 20;
const MAX_ENTRIES: usize = 20_000;
const MAX_CHAPTERS: usize = 5000;
const MAX_TOC: usize = 5000;
/// Запись журнала с местом чтения: по одной на книгу полки.
pub const READING: &str = "reading";
/// Записи журнала с закладками и цитатами.
pub const MARK: &str = "mark";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookFormat {
    Epub,
    Fb2,
}

/// Что известно о книге из самого файла.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Meta {
    pub title: String,
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    /// Аннотация простым текстом.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub genres: Vec<String>,
}

/// Глава в порядке чтения; `size` — знаков текста, по нему считается процент прочитанного.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chapter {
    pub title: String,
    pub size: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TocItem {
    pub title: String,
    pub chapter: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    pub depth: u8,
}

/// Найденное в книге; места — в единицах UTF-16, как их считает страница.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hit {
    pub chapter: usize,
    pub offset: usize,
    pub len: usize,
    pub before: String,
    pub found: String,
    pub after: String,
}

/// Открытая книга.
pub struct Book {
    pub format: BookFormat,
    pub meta: Meta,
    pub chapters: Vec<Chapter>,
    pub toc: Vec<TocItem>,
    html: Vec<String>,
    text: Vec<String>,
    /// Где в тексте главы начинаются и кончаются блоки (для отрывков поиска).
    breaks: Vec<Vec<usize>>,
    res: Res,
}

enum Res {
    Epub { zip: Zip, cover: Option<String> },
    Fb2(fb2::Binaries),
}

impl Book {
    /// Книга из байтов файла: EPUB, FB2 или FB2 в архиве ZIP.
    pub fn open(data: Vec<u8>) -> Result<Book> {
        if data.len() > MAX_BOOK {
            return Err(bad(format!("Книга больше {} МБ — такую не открыть", MAX_BOOK >> 20)));
        }
        if data.starts_with(b"PK\x03\x04") {
            let zip = Zip::new(data)?;
            if zip.lookup("META-INF/container.xml").is_some() {
                return epub::open(zip);
            }
            if let Some(name) = zip.names().into_iter().find(|n| n.to_ascii_lowercase().ends_with(".fb2")) {
                return fb2::open(&zip.read(&name)?);
            }
            return Err(bad("В архиве нет книги EPUB или FB2"));
        }
        if fb2::looks_like(&data) {
            return fb2::open(&data);
        }
        Err(bad("Это не книга EPUB или FB2"))
    }

    /// Глава для страницы: JSON дерева (см. `tree`).
    pub fn chapter(&self, i: usize) -> Option<&str> {
        self.html.get(i).map(String::as_str)
    }

    /// Знаков во всей книге.
    pub fn total(&self) -> usize {
        self.chapters.iter().map(|c| c.size).sum()
    }

    /// Картинка книги по пути из главы; другие файлы не отдаются.
    pub fn image(&self, path: &str) -> Option<(&'static str, Vec<u8>)> {
        let data = match &self.res {
            Res::Epub { zip, .. } => zip.read(&zip.lookup(path)?).ok()?,
            Res::Fb2(b) => b.get(path)?,
        };
        let mime = image_type(&data, path)?;
        Some((mime, data))
    }

    /// Обложка из файла (JPEG, PNG или WebP — то, что примет база).
    pub fn cover(&self) -> Option<(&'static str, Vec<u8>)> {
        let data = match &self.res {
            Res::Epub { zip, cover } => zip.read(cover.as_deref()?).ok()?,
            Res::Fb2(b) => b.cover()?,
        };
        let mime = crate::db::sniff_image(&data)?;
        crate::db::check_blob(mime, &data).ok()?;
        Some((mime, data))
    }

    /// Поиск по тексту без учёта регистра и разницы «е» и «ё»; не больше `limit` мест.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let needle: Vec<char> = squash(query).chars().map(fold).collect();
        let mut hits = Vec::new();
        if needle.len() < 2 {
            return hits;
        }
        for (ci, text) in self.text.iter().enumerate() {
            let chars: Vec<char> = text.chars().collect();
            if chars.len() < needle.len() {
                continue;
            }
            let breaks = &self.breaks[ci];
            let at_break = |i: usize| breaks.binary_search(&i).is_ok();
            // Отрывок с пробелами на границах абзацев.
            let piece = |from: usize, to: usize| -> String {
                let mut s = String::new();
                for (i, &c) in chars.iter().enumerate().take(to).skip(from) {
                    if i > from && at_break(i) && !s.ends_with(' ') && c != ' ' {
                        s.push(' ');
                    }
                    s.push(c);
                }
                s
            };
            let folded: Vec<char> = chars.iter().map(|&c| fold(c)).collect();
            let mut u16_at = 0usize;
            let mut counted = 0usize;
            let mut i = 0;
            while i + needle.len() <= folded.len() {
                if folded[i] == needle[0] && folded[i..i + needle.len()] == needle[..] {
                    u16_at += chars[counted..i].iter().map(|c| c.len_utf16()).sum::<usize>();
                    counted = i;
                    let end = i + needle.len();
                    let mut before = piece(i.saturating_sub(40), i);
                    if at_break(i) && !before.is_empty() && !before.ends_with(' ') {
                        before.push(' ');
                    }
                    let mut after = piece(end, (end + 60).min(chars.len()));
                    if at_break(end) && !after.is_empty() && !after.starts_with(' ') {
                        after.insert(0, ' ');
                    }
                    hits.push(Hit {
                        chapter: ci,
                        offset: u16_at,
                        len: chars[i..end].iter().map(|c| c.len_utf16()).sum(),
                        before,
                        found: piece(i, end),
                        after,
                    });
                    if hits.len() >= limit {
                        return hits;
                    }
                    i = end;
                } else {
                    i += 1;
                }
            }
        }
        hits
    }
}

fn fold(c: char) -> char {
    match c {
        'ё' | 'Ё' => 'е',
        '\u{a0}' | '\t' | '\n' | '\r' => ' ',
        _ => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Глава больше этого (в знаках) делится на части — иначе страница раскладывает её подолгу.
const BIG: usize = 120_000;
/// Примерный объём части большой главы.
const PART: usize = 60_000;

fn chars_of(nodes: &[Out]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Out::Text(t) => t.chars().count(),
            Out::El(e) => chars_of(&e.kids),
        })
        .sum()
}

fn collect_ids(nodes: &[Out], out: &mut Vec<String>) {
    for n in nodes {
        if let Out::El(e) = n {
            if let Some(id) = e.get("id") {
                out.push(id.to_string());
            }
            collect_ids(&e.kids, out);
        }
    }
}

/// Книга одним файлом часто завёрнута в один-два блока: идём внутрь, пока снаружи
/// один большой блок; его метка остаётся пустым якорем.
fn unwrap(mut nodes: Vec<Out>) -> Vec<Out> {
    for _ in 0..6 {
        let [Out::El(e)] = nodes.as_slice() else { break };
        if !matches!(e.tag, "div" | "section" | "blockquote") || e.kids.len() < 2 {
            break;
        }
        let Some(Out::El(wrapper)) = nodes.pop() else { break };
        let mut flat = Vec::with_capacity(wrapper.kids.len() + 1);
        if let Some(id) = wrapper.get("id") {
            flat.push(Out::El(tree::OutEl::new("span").with("id", id)));
        }
        flat.extend(wrapper.kids);
        nodes = flat;
    }
    nodes
}

/// Большие главы — на части по блокам; ссылки и оглавление переводятся на новые номера.
fn split_big(parts: Vec<(String, Vec<Out>)>, toc: &mut [TocItem]) -> Vec<(String, Vec<Out>)> {
    if parts.iter().all(|(_, n)| chars_of(n) <= BIG) {
        return parts;
    }
    let mut out: Vec<(String, Vec<Out>)> = Vec::new();
    let mut first: Vec<usize> = Vec::with_capacity(parts.len());
    let mut anchors: HashMap<(usize, String), usize> = HashMap::new();
    let mut place = |old: usize, title: &str, nodes: Vec<Out>, out: &mut Vec<(String, Vec<Out>)>| {
        let mut ids = Vec::new();
        collect_ids(&nodes, &mut ids);
        for id in ids {
            anchors.entry((old, id)).or_insert(out.len());
        }
        out.push((title.to_string(), nodes));
    };
    for (old, (title, nodes)) in parts.into_iter().enumerate() {
        first.push(out.len());
        if chars_of(&nodes) <= BIG || out.len() >= MAX_CHAPTERS {
            place(old, &title, nodes, &mut out);
            continue;
        }
        let mut cur: Vec<Out> = Vec::new();
        let mut size = 0;
        for n in unwrap(nodes) {
            let s = chars_of(std::slice::from_ref(&n));
            if size > 0 && size + s > PART {
                place(old, &title, std::mem::take(&mut cur), &mut out);
                size = 0;
            }
            size += s;
            cur.push(n);
        }
        if !cur.is_empty() {
            place(old, &title, cur, &mut out);
        }
    }
    let remap = |to: &str| -> Option<String> {
        let (ch, anchor) = match to.split_once('#') {
            Some((c, a)) => (c.parse::<usize>().ok()?, Some(a)),
            None => (to.parse::<usize>().ok()?, None),
        };
        let start = *first.get(ch)?;
        Some(match anchor {
            Some(a) => format!("{}#{a}", anchors.get(&(ch, a.to_string())).copied().unwrap_or(start)),
            None => start.to_string(),
        })
    };
    for (_, nodes) in &mut out {
        tree::walk_mut(nodes, &mut |el| {
            if let Some(to) = el.get("to").map(str::to_string) {
                match remap(&to) {
                    Some(new) => el.set("to", new),
                    None => el.remove("to"),
                }
            }
        });
    }
    for t in toc.iter_mut() {
        let start = first.get(t.chapter).copied().unwrap_or(0);
        t.chapter = t.anchor.as_ref().and_then(|a| anchors.get(&(t.chapter, a.clone())).copied()).unwrap_or(start);
    }
    out
}

/// Собирает книгу из глав-деревьев: JSON для страницы, текст для поиска и объём.
fn finish(format: BookFormat, meta: Meta, parts: Vec<(String, Vec<Out>)>, mut toc: Vec<TocItem>, res: Res) -> Result<Book> {
    if parts.is_empty() {
        return Err(bad("В книге нет текста"));
    }
    let parts = split_big(parts, &mut toc);
    let mut chapters = Vec::with_capacity(parts.len());
    let mut html = Vec::with_capacity(parts.len());
    let mut text = Vec::with_capacity(parts.len());
    let mut breaks = Vec::with_capacity(parts.len());
    for (title, nodes) in &parts {
        let mut t = String::new();
        let mut b = Vec::new();
        let mut size = 0;
        tree::text_and_breaks(nodes, &mut t, &mut b, &mut size);
        chapters.push(Chapter { title: title.clone(), size });
        html.push(tree::to_json(nodes));
        text.push(t);
        breaks.push(b);
    }
    if chapters.iter().all(|c| c.size == 0) && !parts.iter().any(|(_, n)| has_image(n)) {
        return Err(bad("В книге нет текста"));
    }
    toc.retain(|t| t.chapter < chapters.len());
    toc.truncate(MAX_TOC);
    Ok(Book { format, meta, chapters, toc, html, text, breaks, res })
}

fn has_image(nodes: &[Out]) -> bool {
    nodes.iter().any(|n| matches!(n, Out::El(e) if e.tag == "img" || has_image(&e.kids)))
}

/// Тип картинки по первым байтам (SVG — по имени файла: внутри `<img>` скрипты не работают).
fn image_type(data: &[u8], path: &str) -> Option<&'static str> {
    if let Some(m) = crate::db::sniff_image(data) {
        return Some(m);
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if path.to_ascii_lowercase().ends_with(".svg") {
        return Some("image/svg+xml");
    }
    None
}

/// Архив ZIP в памяти: записи по имени (и без учёта регистра), с пределами размера.
pub(crate) struct Zip {
    archive: Mutex<zip::ZipArchive<Cursor<Vec<u8>>>>,
    lower: HashMap<String, String>,
}

impl Zip {
    pub fn new(data: Vec<u8>) -> Result<Zip> {
        let archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|_| bad("Архив книги повреждён"))?;
        if archive.len() > MAX_ENTRIES {
            return Err(bad("В архиве книги слишком много файлов"));
        }
        let lower = archive.file_names().filter_map(|n| n.ok()).map(|n| (n.to_lowercase(), n.into_owned())).collect();
        Ok(Zip { archive: Mutex::new(archive), lower })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, zip::ZipArchive<Cursor<Vec<u8>>>> {
        self.archive.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn names(&self) -> Vec<String> {
        self.lock().file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect()
    }

    /// Точное имя записи: как есть или без учёта регистра.
    pub fn lookup(&self, name: &str) -> Option<String> {
        if self.lock().index_for_name(name).is_some() {
            return Some(name.to_string());
        }
        self.lower.get(&name.to_lowercase()).cloned()
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let mut a = self.lock();
        let f = a.by_name(name).map_err(|_| bad(format!("в книге нет файла {name}")))?;
        if f.size() > MAX_ENTRY {
            return Err(bad("файл в книге слишком большой"));
        }
        let mut out = Vec::with_capacity(f.size() as usize);
        f.take(MAX_ENTRY + 1).read_to_end(&mut out).map_err(|_| bad("файл в книге повреждён"))?;
        if out.len() as u64 > MAX_ENTRY {
            return Err(bad("файл в книге слишком большой"));
        }
        Ok(out)
    }
}

/// Текст без лишних пробелов и переносов.
pub(crate) fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Не длиннее `max` знаков.
pub(crate) fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", s[..at].trim_end()),
        None => s.to_string(),
    }
}

/// Год из даты: «2021-05-01» → «2021».
pub(crate) fn year_of(s: &str) -> Option<String> {
    let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    (digits.len() == 4).then_some(digits)
}

/// Схема адреса: «https:», «mailto:» — у путей внутри книги её нет.
pub(crate) fn scheme(href: &str) -> Option<String> {
    let colon = href.find(':')?;
    let s = &href[..colon];
    let first = s.chars().next()?;
    (first.is_ascii_alphabetic() && s.len() >= 2 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then(|| s.to_ascii_lowercase())
}

/// Раскодирует `%D0%81` и подобное; неверные последовательности остаются как есть.
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Путь внутри книги по ссылке из файла `base`: `../Images/a.jpg#x` → (`OEBPS/Images/a.jpg`, `x`).
/// Внешние адреса и выход за корень архива — `None`.
pub(crate) fn resolve(base: &str, href: &str) -> Option<(String, Option<String>)> {
    let href = href.trim();
    if href.is_empty() || scheme(href).is_some() || href.starts_with("//") {
        return None;
    }
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(percent_decode(f)).filter(|f| !f.is_empty())),
        None => (href, None),
    };
    let path = path.split('?').next().unwrap_or("");
    let path = percent_decode(path);
    let joined = if path.is_empty() {
        base.to_string()
    } else if let Some(abs) = path.strip_prefix('/') {
        abs.to_string()
    } else {
        match base.rfind('/') {
            Some(p) => format!("{}/{}", &base[..p], path),
            None => path,
        }
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some((parts.join("/"), frag))
}

/// Где читатель остановился — запись журнала `reading` с номером записи полки.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    /// Номер записи на полке.
    pub title: String,
    /// SHA-256 файла книги: в том же файле место точное, в другом — по проценту.
    pub book: String,
    pub chapter: u32,
    /// Знаков от начала главы (в единицах UTF-16, как считает страница).
    pub offset: u32,
    /// Прочитано, от 0 до 100.
    pub percent: f32,
    /// Когда читали, секунды Unix.
    pub at: i64,
    /// Минут чтения этой книги — для «Итогов».
    #[serde(default)]
    pub minutes: u32,
    /// Те же минуты по годам: «2026» → минуты.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub years: BTreeMap<String, u32>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Reading {
    pub fn check(&self) -> Result<()> {
        if self.title.is_empty() || self.title.len() > 64 || !hex64(&self.book) {
            return Err(Error::Format("неверная запись о чтении".into()));
        }
        if !self.percent.is_finite() || !(0.0..=100.0).contains(&self.percent) || self.chapter as usize > MAX_CHAPTERS {
            return Err(Error::Format("неверное место в книге".into()));
        }
        let year = |y: &str| y.len() == 4 && y.bytes().all(|b| b.is_ascii_digit());
        if self.years.len() > 100 || self.years.iter().any(|(y, m)| !year(y) || *m > 1_000_000) {
            return Err(Error::Format("неверное время чтения".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkKind {
    Bookmark,
    Quote,
}

/// Закладка или цитата — запись журнала `mark`: обменивается между устройствами, как полка.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub id: String,
    /// Запись полки.
    pub title: String,
    /// SHA-256 файла книги.
    pub book: String,
    pub kind: MarkKind,
    pub chapter: u32,
    /// Начало (знаков от начала главы, UTF-16).
    pub offset: u32,
    /// Конец цитаты (не включая).
    #[serde(default)]
    pub end: u32,
    /// Текст цитаты; у закладки — начало страницы.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Своя заметка к цитате.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Где в книге, от 0 до 100.
    pub percent: f32,
    /// Когда сделана, секунды Unix.
    pub created: i64,
}

impl Mark {
    pub fn check(&self) -> Result<()> {
        let id_ok = (1..=32).contains(&self.id.len()) && self.id.bytes().all(|b| b.is_ascii_alphanumeric());
        if !id_ok || self.title.is_empty() || self.title.len() > 64 || !hex64(&self.book) {
            return Err(Error::Format("неверная закладка".into()));
        }
        if self.text.chars().count() > 4000 {
            return Err(Error::Format("Цитата длиннее 4000 знаков — выделите поменьше".into()));
        }
        if self.note.chars().count() > 2000 {
            return Err(Error::Format("Заметка к цитате длиннее 2000 знаков".into()));
        }
        if !self.percent.is_finite() || !(0.0..=100.0).contains(&self.percent) || self.chapter as usize > MAX_CHAPTERS {
            return Err(Error::Format("неверное место в книге".into()));
        }
        if self.kind == MarkKind::Quote && (self.end <= self.offset || self.text.trim().is_empty()) {
            return Err(Error::Format("Цитата пустая — выделите текст".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests;

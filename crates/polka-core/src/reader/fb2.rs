//! FictionBook 2: описание из `<description>`, главы — разделы основного `<body>`,
//! примечания — из `<body name="notes">`, картинки — из `<binary>` (base64).
//!
//! Большие разделы с подразделами («Часть первая» → главы) делятся на главы по подразделам
//! (до двух уровней), глубже — остаются внутри главы и попадают в оглавление метками.

use super::html::ok_id;
use super::markup::{self, decode_xml, El, Node};
use super::tree::{is_block, push_text, tidy, walk_mut, Out, OutEl};
use super::{bad, clip, finish, scheme, squash, year_of, Book, BookFormat, Meta, Res, Result, TocItem, MAX_CHAPTERS, MAX_TOC};
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;
use std::collections::HashMap;

const B64: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::STANDARD, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent));
const MAX_BINARY: usize = 40 << 20;

/// Картинки книги: номер → base64; расшифровываются, когда их просят.
pub(super) struct Binaries {
    map: HashMap<String, String>,
    cover: Option<String>,
}

impl Binaries {
    pub(super) fn get(&self, id: &str) -> Option<Vec<u8>> {
        let b64 = self.map.get(id.trim_start_matches('#'))?;
        let clean: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        B64.decode(clean).ok()
    }

    pub(super) fn cover(&self) -> Option<Vec<u8>> {
        self.get(self.cover.as_deref()?)
    }
}

/// Похоже на FB2: корень `<FictionBook` в начале файла.
pub fn looks_like(data: &[u8]) -> bool {
    let head = &data[..data.len().min(4096)];
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return decode_xml(head).contains("FictionBook");
    }
    head.windows(11).any(|w| w == b"FictionBook")
}

pub(super) fn open(data: &[u8]) -> Result<Book> {
    let doc = markup::parse(&decode_xml(data), false);
    let fb = doc.find("fictionbook").ok_or_else(|| bad("Это не книга FB2"))?;
    let meta = meta(fb);
    let mut map = HashMap::new();
    for b in fb.children("binary") {
        if let Some(id) = b.attr("id") {
            let data = b.text();
            if data.len() <= MAX_BINARY {
                map.insert(id.trim().to_string(), data);
            }
        }
    }
    let cover = fb
        .find("description")
        .and_then(|d| d.child("title-info"))
        .and_then(|t| t.child("coverpage"))
        .and_then(|c| c.find("image"))
        .and_then(|i| i.attr("href"))
        .map(|h| h.trim().trim_start_matches('#').to_string());
    let bodies: Vec<&El> = fb.children("body").collect();
    let main_at = bodies.iter().position(|b| !is_notes(b)).unwrap_or(0);
    let mut b = Builder { binaries: &map, parts: Vec::new(), toc: Vec::new(), anchors: HashMap::new(), current: 0, depth: 0, toc_on: true, auto: 0 };
    if let Some(main) = bodies.get(main_at) {
        b.main(main);
    }
    for (i, body) in bodies.iter().enumerate() {
        if i != main_at {
            b.notes(body);
        }
    }
    b.links();
    let Builder { parts, toc, .. } = b;
    finish(BookFormat::Fb2, meta, parts, toc, Res::Fb2(Binaries { map, cover }))
}

fn is_notes(body: &El) -> bool {
    body.attr("name").is_some_and(|n| matches!(n.to_lowercase().as_str(), "notes" | "comments" | "footnotes"))
}

fn is_section(k: &Node) -> bool {
    matches!(k, Node::El(e) if e.name == "section")
}

fn has_content(nodes: &[&Node]) -> bool {
    nodes.iter().any(|k| match k {
        Node::El(_) => true,
        Node::Text(t) => !t.trim().is_empty(),
    })
}

/// Название раздела одной строкой: абзацы `<title>` через точку.
fn title_text(t: &El) -> String {
    let mut parts: Vec<String> = t.children("p").map(|p| squash(&p.text())).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        let s = squash(&t.text());
        if !s.is_empty() {
            parts.push(s);
        }
    }
    let mut out = String::new();
    for p in parts {
        if !out.is_empty() {
            out.push_str(if out.ends_with(['.', '!', '?', ':', ';', '…']) { " " } else { ". " });
        }
        out.push_str(&p);
    }
    out
}

struct Builder<'a> {
    binaries: &'a HashMap<String, String>,
    parts: Vec<(String, Vec<Out>)>,
    toc: Vec<TocItem>,
    /// Метка (id) → глава: для сносок и оглавления.
    anchors: HashMap<String, usize>,
    /// Глава, которая сейчас собирается.
    current: usize,
    /// Уровень текущей главы в оглавлении.
    depth: u8,
    /// В примечаниях подразделы в оглавление не идут.
    toc_on: bool,
    /// Счётчик своих меток для разделов без id.
    auto: usize,
}

impl Builder<'_> {
    fn push(&mut self, title: String, mut out: Vec<Out>) {
        tidy(&mut out);
        self.parts.push((title, out));
    }

    fn anchor(&mut self, id: &str) {
        if ok_id(id) {
            self.anchors.entry(id.to_string()).or_insert(self.current);
        }
    }

    fn toc_item(&mut self, title: &str, anchor: Option<String>, depth: u8) {
        if self.toc_on && !title.is_empty() && self.toc.len() < MAX_TOC {
            self.toc.push(TocItem { title: clip(title, 200), chapter: self.current, anchor, depth });
        }
    }

    fn main(&mut self, body: &El) {
        // Всё до первого раздела — заглавная страница: картинка, название, эпиграф.
        let head: Vec<&Node> = body.kids.iter().take_while(|k| !is_section(k)).collect();
        if has_content(&head) {
            self.current = self.parts.len();
            let mut out = Vec::new();
            for k in head {
                self.node(k, 0, &mut out);
            }
            let title = body.child("title").map(title_text).unwrap_or_default();
            self.push(title, out);
        }
        for sec in body.children("section") {
            self.section(sec, 0);
        }
    }

    fn section(&mut self, sec: &El, depth: u8) {
        if self.parts.len() >= MAX_CHAPTERS {
            return;
        }
        let title = sec.child("title").map(title_text).unwrap_or_default();
        self.current = self.parts.len();
        self.depth = depth;
        if depth < 2 && sec.children("section").next().is_some() {
            // Голова раздела (название, эпиграф) — короткой главой, подразделы — своими главами.
            if let Some(id) = sec.attr("id") {
                self.anchor(id);
            }
            self.toc_item(&title, None, depth);
            let head: Vec<&Node> = sec.kids.iter().take_while(|k| !is_section(k)).collect();
            if has_content(&head) {
                let mut el = OutEl::new("section");
                if let Some(id) = sec.attr("id").filter(|i| ok_id(i)) {
                    el.set("id", id);
                }
                for k in head {
                    self.node(k, 1, &mut el.kids);
                }
                tidy(&mut el.kids);
                self.push(title, vec![Out::El(el)]);
            }
            let mut after = false;
            for k in &sec.kids {
                if is_section(k) {
                    if let Node::El(e) = k {
                        self.section(e, depth + 1);
                    }
                    after = true;
                } else if after && !self.parts.is_empty() {
                    // Текст после подразделов — в конец последней главы.
                    let last = self.parts.len() - 1;
                    self.current = last;
                    let mut extra = Vec::new();
                    self.node(k, 1, &mut extra);
                    self.parts[last].1.extend(extra);
                }
            }
        } else {
            self.toc_item(&title, None, depth);
            let mut out = Vec::new();
            self.conv(sec, 0, &mut out);
            self.push(title, out);
        }
    }

    /// Примечания и другие тела книги — по одной главе, без подразделов в оглавлении.
    fn notes(&mut self, body: &El) {
        if self.parts.len() >= MAX_CHAPTERS {
            return;
        }
        self.current = self.parts.len();
        let comments = body.attr("name").is_some_and(|n| n.eq_ignore_ascii_case("comments"));
        let title = body.child("title").map(title_text).filter(|t| !t.is_empty()).unwrap_or_else(|| (if comments { "Комментарии" } else { "Примечания" }).to_string());
        self.toc_item(&title, None, 0);
        let was = self.toc_on;
        self.toc_on = false;
        let mut heading = OutEl::new("h1");
        heading.kids.push(Out::Text(title.clone()));
        let mut out = vec![Out::El(heading)];
        for k in &body.kids {
            if !matches!(k, Node::El(e) if e.name == "title") {
                self.node(k, 2, &mut out);
            }
        }
        self.toc_on = was;
        self.push(title, out);
    }

    fn node(&mut self, k: &Node, level: u8, out: &mut Vec<Out>) {
        match k {
            Node::El(e) => self.conv(e, level, out),
            Node::Text(t) => {
                if !t.trim().is_empty() {
                    push_text(out, t, false);
                }
            }
        }
    }

    fn conv(&mut self, e: &El, level: u8, out: &mut Vec<Out>) {
        if let Some(id) = e.attr("id") {
            self.anchor(id);
        }
        let mut el = match e.name.as_str() {
            "section" => {
                let mut el = OutEl::new("section");
                if level > 0 {
                    let title = e.child("title").map(title_text).unwrap_or_default();
                    if !title.is_empty() && self.toc_on && self.toc.len() < MAX_TOC {
                        let anchor = match e.attr("id").filter(|i| ok_id(i)) {
                            Some(id) => id.to_string(),
                            None => {
                                self.auto += 1;
                                let id = format!("polka-{}", self.auto);
                                el.set("id", id.clone());
                                id
                            }
                        };
                        let depth = self.depth.saturating_add(level);
                        self.toc_item(&title, Some(anchor), depth);
                    }
                }
                for k in &e.kids {
                    self.node(k, level.saturating_add(1), &mut el.kids);
                }
                el
            }
            "title" => {
                let tag = match level {
                    0 => "h1",
                    1 => "h2",
                    2 => "h3",
                    _ => "h4",
                };
                let mut el = OutEl::new(tag);
                let mut first = true;
                for k in e.elements() {
                    match k.name.as_str() {
                        "p" => {
                            if !first {
                                el.kids.push(Out::El(OutEl::new("br")));
                            }
                            if let Some(id) = k.attr("id") {
                                self.anchor(id);
                            }
                            self.inline(&k.kids, &mut el.kids);
                            first = false;
                        }
                        "empty-line" => el.kids.push(Out::El(OutEl::new("br"))),
                        _ => {}
                    }
                }
                el
            }
            "p" => self.para(None, e),
            "subtitle" => self.para(Some("subtitle"), e),
            "v" => self.para(Some("verse"), e),
            "text-author" => self.para(Some("author"), e),
            "date" => self.para(Some("date"), e),
            "empty-line" => OutEl::new("div").with("class", "gap"),
            "epigraph" => self.block("blockquote", "epigraph", e, level),
            "cite" => self.block("blockquote", "cite", e, level),
            "poem" => self.block("div", "poem", e, level),
            "stanza" => self.block("div", "stanza", e, level),
            "annotation" => self.block("div", "annotation", e, level),
            "image" => match self.image(e) {
                Some(img) => {
                    let mut d = OutEl::new("div").with("class", "image");
                    d.kids.push(img);
                    d
                }
                None => return,
            },
            "table" => self.table(e),
            "strong" | "emphasis" | "a" | "strikethrough" | "sub" | "sup" | "code" | "style" => {
                // Строчная разметка вне абзаца — оборачиваем в абзац.
                let mut p = OutEl::new("p");
                self.inline_el(e, &mut p.kids);
                p
            }
            _ => {
                for k in &e.kids {
                    self.node(k, level, out);
                }
                return;
            }
        };
        if let Some(id) = e.attr("id").filter(|i| ok_id(i)) {
            if el.get("id").is_none() {
                el.set("id", id);
            }
        }
        if is_block(el.tag) {
            tidy(&mut el.kids);
        }
        out.push(Out::El(el));
    }

    fn para(&mut self, class: Option<&'static str>, e: &El) -> OutEl {
        let mut el = OutEl::new("p");
        if let Some(c) = class {
            el.set("class", c);
        }
        self.inline(&e.kids, &mut el.kids);
        el
    }

    fn block(&mut self, tag: &'static str, class: &'static str, e: &El, level: u8) -> OutEl {
        let mut el = OutEl::new(tag).with("class", class);
        for k in &e.kids {
            self.node(k, level.saturating_add(1), &mut el.kids);
        }
        el
    }

    fn inline(&mut self, nodes: &[Node], out: &mut Vec<Out>) {
        for k in nodes {
            match k {
                Node::Text(t) => push_text(out, t, false),
                Node::El(e) => self.inline_el(e, out),
            }
        }
    }

    fn inline_el(&mut self, e: &El, out: &mut Vec<Out>) {
        if let Some(id) = e.attr("id") {
            self.anchor(id);
        }
        let tag = match e.name.as_str() {
            "strong" => "strong",
            "emphasis" => "em",
            "strikethrough" => "s",
            "sub" => "sub",
            "sup" => "sup",
            "code" => "code",
            "style" => "span",
            "a" => "a",
            "image" => {
                if let Some(img) = self.image(e) {
                    out.push(img);
                }
                return;
            }
            _ => {
                self.inline(&e.kids, out);
                return;
            }
        };
        let mut el = OutEl::new(tag);
        if tag == "a" {
            if let Some(href) = e.attr("href").map(str::trim) {
                if let Some(anchor) = href.strip_prefix('#') {
                    if !anchor.is_empty() {
                        el.set("to", format!("#{anchor}"));
                    }
                } else if matches!(scheme(href).as_deref(), Some("http" | "https" | "mailto")) && href.len() <= 2000 {
                    el.set("ext", href);
                }
            }
            if e.attr("type") == Some("note") {
                el.set("note", "1");
            }
        }
        if let Some(id) = e.attr("id").filter(|i| ok_id(i)) {
            el.set("id", id);
        }
        self.inline(&e.kids, &mut el.kids);
        if el.kids.is_empty() && el.get("id").is_none() {
            return;
        }
        out.push(Out::El(el));
    }

    fn image(&self, e: &El) -> Option<Out> {
        let id = e.attr("href")?.trim().trim_start_matches('#');
        if !self.binaries.contains_key(id) {
            return None;
        }
        let mut img = OutEl::new("img").with("src", id);
        if let Some(alt) = e.attr("alt").map(squash).filter(|a| !a.is_empty()) {
            img.set("alt", clip(&alt, 200));
        }
        Some(Out::El(img))
    }

    fn table(&mut self, e: &El) -> OutEl {
        let mut table = OutEl::new("table");
        for tr in e.children("tr") {
            let mut row = OutEl::new("tr");
            for cell in tr.elements().filter(|c| c.name == "td" || c.name == "th") {
                let mut c = OutEl::new(if cell.name == "th" { "th" } else { "td" });
                for key in ["colspan", "rowspan"] {
                    if let Some(n) = cell.attr(key).and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| (2..=50).contains(n)) {
                        c.set(key, n.to_string());
                    }
                }
                if let Some(a) = cell.attr("align").filter(|a| matches!(*a, "left" | "right" | "center")) {
                    c.set("style", format!("text-align:{a}"));
                }
                self.inline(&cell.kids, &mut c.kids);
                tidy(&mut c.kids);
                row.kids.push(Out::El(c));
            }
            table.kids.push(Out::El(row));
        }
        table
    }

    /// Ссылки на метки (`#n1`) → глава и метка; на несуществующие — без перехода.
    fn links(&mut self) {
        let anchors = &self.anchors;
        for (_, nodes) in &mut self.parts {
            walk_mut(nodes, &mut |el| {
                if el.tag != "a" {
                    return;
                }
                if let Some(id) = el.get("to").and_then(|t| t.strip_prefix('#')).map(str::to_string) {
                    match anchors.get(&id) {
                        Some(ch) => el.set("to", format!("{ch}#{id}")),
                        None => el.remove("to"),
                    }
                }
            });
        }
    }
}

fn meta(fb: &El) -> Meta {
    let desc = fb.child("description");
    let ti = desc.and_then(|d| d.child("title-info"));
    let pi = desc.and_then(|d| d.child("publish-info"));
    let text = |e: Option<&El>| e.map(|e| squash(&e.text())).filter(|s| !s.is_empty());
    let title = text(ti.and_then(|t| t.child("book-title"))).or_else(|| text(pi.and_then(|p| p.child("book-name"))));
    let year = ti
        .and_then(|t| t.child("date"))
        .and_then(|d| d.attr("value").and_then(year_of).or_else(|| year_of(&d.text())))
        .or_else(|| pi.and_then(|p| p.child("year")).and_then(|y| year_of(&y.text())));
    let mut genres: Vec<String> = Vec::new();
    for g in ti.into_iter().flat_map(|t| t.children("genre")) {
        if let Some(name) = genre(&squash(&g.text())) {
            if !genres.iter().any(|x| x == name) {
                genres.push(name.to_string());
            }
        }
    }
    genres.truncate(5);
    Meta {
        title: title.map(|t| clip(&t, 300)).unwrap_or_default(),
        authors: ti.into_iter().flat_map(|t| t.children("author")).filter_map(person).take(10).collect(),
        lang: text(ti.and_then(|t| t.child("lang"))),
        year,
        publisher: text(pi.and_then(|p| p.child("publisher"))),
        isbn: pi.and_then(|p| p.child("isbn")).and_then(|i| crate::isbn::normalize(&i.text())),
        about: ti.and_then(|t| t.child("annotation")).map(|a| a.block_text()).filter(|s| !s.is_empty()).map(|s| clip(&s, 3000)),
        genres,
    }
}

/// Автор: имя и фамилия (отчество не берём — на полке его обычно нет), иначе псевдоним.
fn person(a: &El) -> Option<String> {
    let part = |n: &str| a.child(n).map(|e| squash(&e.text())).filter(|s| !s.is_empty());
    let name: Vec<String> = ["first-name", "last-name"].iter().filter_map(|n| part(n)).collect();
    if name.is_empty() {
        part("nickname")
    } else {
        Some(name.join(" "))
    }
}

/// Жанры FB2 — коды вроде `sf_fantasy`; на полке — по-русски.
fn genre(code: &str) -> Option<&'static str> {
    const NAMES: [(&str, &str); 64] = [
        ("sf_fantasy", "Фэнтези"),
        ("sf_horror", "Ужасы"),
        ("sf_humor", "Юмористическая фантастика"),
        ("sf_space", "Космическая фантастика"),
        ("sf_history", "Альтернативная история"),
        ("sf_cyberpunk", "Киберпанк"),
        ("sf_detective", "Детективная фантастика"),
        ("sf_action", "Боевая фантастика"),
        ("sf_heroic", "Героическая фантастика"),
        ("sf_epic", "Эпическая фантастика"),
        ("sf_postapocalyptic", "Постапокалипсис"),
        ("sf_social", "Социальная фантастика"),
        ("sf_mystic", "Мистика"),
        ("det_classic", "Классический детектив"),
        ("det_police", "Полицейский детектив"),
        ("det_irony", "Иронический детектив"),
        ("det_history", "Исторический детектив"),
        ("det_espionage", "Шпионский детектив"),
        ("det_crime", "Криминальный детектив"),
        ("det_political", "Политический детектив"),
        ("det_action", "Боевик"),
        ("thriller", "Триллер"),
        ("prose_classic", "Классическая проза"),
        ("prose_history", "Историческая проза"),
        ("prose_contemporary", "Современная проза"),
        ("prose_counter", "Контркультура"),
        ("prose_rus_classic", "Русская классика"),
        ("prose_su_classics", "Советская классика"),
        ("prose_military", "Военная проза"),
        ("love_contemporary", "Современный любовный роман"),
        ("love_history", "Исторический любовный роман"),
        ("love_detective", "Остросюжетный любовный роман"),
        ("love_short", "Короткий любовный роман"),
        ("adv_western", "Вестерн"),
        ("adv_history", "Исторические приключения"),
        ("adv_maritime", "Морские приключения"),
        ("adv_geo", "Путешествия и география"),
        ("adv_animal", "Природа и животные"),
        ("child_tale", "Сказки"),
        ("child_verse", "Детские стихи"),
        ("child_prose", "Детская проза"),
        ("child_sf", "Детская фантастика"),
        ("child_det", "Детский детектив"),
        ("child_adv", "Детские приключения"),
        ("poetry", "Поэзия"),
        ("dramaturgy", "Драматургия"),
        ("antique_myths", "Мифы и легенды"),
        ("sci_history", "История"),
        ("sci_psychology", "Психология"),
        ("sci_culture", "Культурология"),
        ("sci_religion", "Религиоведение"),
        ("sci_philosophy", "Философия"),
        ("sci_politics", "Политика"),
        ("sci_business", "Деловая литература"),
        ("sci_popular", "Научно-популярное"),
        ("nonf_biography", "Биографии и мемуары"),
        ("nonf_publicism", "Публицистика"),
        ("nonf_criticism", "Критика"),
        ("comp_programming", "Программирование"),
        ("ref_encyc", "Энциклопедии"),
        ("ref_dict", "Словари"),
        ("humor_anecdote", "Анекдоты"),
        ("home_cooking", "Кулинария"),
        ("religion_self", "Самосовершенствование"),
    ];
    const FAMILIES: [(&str, &str); 16] = [
        ("sf", "Фантастика"),
        ("det", "Детектив"),
        ("detective", "Детектив"),
        ("prose", "Проза"),
        ("love", "Любовный роман"),
        ("adv", "Приключения"),
        ("adventure", "Приключения"),
        ("child", "Детское"),
        ("children", "Детское"),
        ("antique", "Старинная литература"),
        ("sci", "Научная литература"),
        ("science", "Научная литература"),
        ("comp", "Компьютеры"),
        ("nonf", "Документальная литература"),
        ("humor", "Юмор"),
        ("religion", "Религия"),
    ];
    let code = code.trim().to_ascii_lowercase();
    if let Some((_, name)) = NAMES.iter().find(|(c, _)| *c == code) {
        return Some(name);
    }
    let family = code.split('_').next().unwrap_or("");
    FAMILIES.iter().find(|(c, _)| *c == family).map(|(_, n)| *n)
}

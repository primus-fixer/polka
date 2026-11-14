//! Глава, готовая к показу: дерево только из разрешённых тегов и атрибутов.
//!
//! Страница строит его через `createElement` — разметка книги в `innerHTML` не попадает.
//! В JSON узел — строка (текст) или `[тег, {атрибуты}, …дети]`.
//! Атрибуты: `id`, `class` (только свои имена из FB2), `style` (собран из разрешённых значений),
//! `src` и `alt` у картинок (путь внутри книги), `to` (глава и метка: `3#n12`), `ext` (адрес
//! http, https или mailto), `note` (сноска), `colspan`, `rowspan`, `start`.

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Text(String),
    El(OutEl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutEl {
    pub tag: &'static str,
    pub attrs: Vec<(&'static str, String)>,
    pub kids: Vec<Out>,
}

impl OutEl {
    pub fn new(tag: &'static str) -> OutEl {
        OutEl { tag, attrs: Vec::new(), kids: Vec::new() }
    }

    pub fn with(mut self, key: &'static str, value: impl Into<String>) -> OutEl {
        self.set(key, value);
        self
    }

    pub fn set(&mut self, key: &'static str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.attrs.push((key, value)),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    pub fn remove(&mut self, key: &str) {
        self.attrs.retain(|(k, _)| *k != key);
    }
}

impl Serialize for Out {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Out::Text(t) => s.serialize_str(t),
            Out::El(e) => {
                let mut seq = s.serialize_seq(Some(2 + e.kids.len()))?;
                seq.serialize_element(e.tag)?;
                seq.serialize_element(&Attrs(&e.attrs))?;
                for k in &e.kids {
                    seq.serialize_element(k)?;
                }
                seq.end()
            }
        }
    }
}

struct Attrs<'a>(&'a [(&'static str, String)]);

impl Serialize for Attrs<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

/// Блочные теги: пробелы между ними ничего не значат.
pub fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "th"
            | "td"
            | "caption"
            | "figure"
            | "figcaption"
            | "section"
            | "aside"
            | "hr"
    )
}

/// Добавляет текст: пробелы и переносы схлопываются в один пробел (кроме `pre`),
/// соседние куски текста сливаются.
pub fn push_text(out: &mut Vec<Out>, text: &str, pre: bool) {
    if text.is_empty() {
        return;
    }
    let mut add = String::with_capacity(text.len());
    if pre {
        add.push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
    } else {
        let mut space = matches!(out.last(), Some(Out::Text(t)) if t.ends_with(' '));
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{0C}') {
                if !space {
                    add.push(' ');
                    space = true;
                }
            } else {
                add.push(c);
                space = false;
            }
        }
    }
    if add.is_empty() {
        return;
    }
    if let Some(Out::Text(last)) = out.last_mut() {
        last.push_str(&add);
    } else {
        out.push(Out::Text(add));
    }
}

fn blank(t: &str) -> bool {
    t.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C))
}

/// Уборка внутри блока: пробельный текст рядом с блоками и по краям не нужен,
/// пробелы в начале и в конце блока — тоже.
pub fn tidy(kids: &mut Vec<Out>) {
    let n = kids.len();
    let is_block_at = |kids: &Vec<Out>, i: usize| matches!(kids.get(i), Some(Out::El(e)) if is_block(e.tag));
    let keep: Vec<bool> = (0..n)
        .map(|i| match &kids[i] {
            Out::Text(t) if blank(t) => {
                let prev = i == 0 || is_block_at(kids, i - 1);
                let next = i + 1 == n || is_block_at(kids, i + 1);
                !(prev || next)
            }
            _ => true,
        })
        .collect();
    let mut it = keep.into_iter();
    kids.retain(|_| it.next().unwrap_or(true));
    if let Some(Out::Text(t)) = kids.first_mut() {
        let trimmed = t.trim_start_matches(' ');
        if trimmed.len() != t.len() {
            *t = trimmed.to_string();
        }
    }
    if let Some(Out::Text(t)) = kids.last_mut() {
        let trimmed = t.trim_end_matches(' ');
        if trimmed.len() != t.len() {
            *t = trimmed.to_string();
        }
    }
    kids.retain(|k| !matches!(k, Out::Text(t) if t.is_empty()));
}

/// Весь текст по порядку — так же, как его сложит страница из текстовых узлов.
pub fn text_of(nodes: &[Out], out: &mut String) {
    for n in nodes {
        match n {
            Out::Text(t) => out.push_str(t),
            Out::El(e) => text_of(&e.kids, out),
        }
    }
}

/// Текст и места (в знаках), где начинаются и кончаются блоки, — в отрывках поиска
/// там ставится пробел, чтобы абзацы не слипались.
pub fn text_and_breaks(nodes: &[Out], out: &mut String, breaks: &mut Vec<usize>, count: &mut usize) {
    for n in nodes {
        match n {
            Out::Text(t) => {
                out.push_str(t);
                *count += t.chars().count();
            }
            Out::El(e) => {
                let block = is_block(e.tag) || e.tag == "br";
                if block && breaks.last() != Some(count) {
                    breaks.push(*count);
                }
                text_and_breaks(&e.kids, out, breaks, count);
                if block && breaks.last() != Some(count) {
                    breaks.push(*count);
                }
            }
        }
    }
}

/// Первый заголовок h1–h3 — название главы, если его нет в оглавлении.
pub fn first_heading(nodes: &[Out]) -> Option<String> {
    for n in nodes {
        if let Out::El(e) = n {
            if matches!(e.tag, "h1" | "h2" | "h3") {
                let mut s = String::new();
                text_of(&e.kids, &mut s);
                let s = super::squash(&s);
                if !s.is_empty() {
                    return Some(super::clip(&s, 120));
                }
            }
            if let Some(h) = first_heading(&e.kids) {
                return Some(h);
            }
        }
    }
    None
}

pub fn to_json(nodes: &[Out]) -> String {
    serde_json::to_string(nodes).unwrap_or_else(|_| "[]".into())
}

/// Обход всех элементов (для поздней правки ссылок).
pub fn walk_mut(nodes: &mut [Out], f: &mut dyn FnMut(&mut OutEl)) {
    for n in nodes {
        if let Out::El(e) = n {
            f(e);
            walk_mut(&mut e.kids, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_collapses_and_merges() {
        let mut out = Vec::new();
        push_text(&mut out, "  раз\n\t два ", false);
        push_text(&mut out, " три", false);
        assert_eq!(out, vec![Out::Text(" раз два три".into())]);
        let mut pre = Vec::new();
        push_text(&mut pre, "a\r\n  b", true);
        assert_eq!(pre, vec![Out::Text("a\n  b".into())]);
    }

    #[test]
    fn tidy_keeps_spaces_between_inline() {
        let em = |t: &str| Out::El(OutEl { tag: "em", attrs: vec![], kids: vec![Out::Text(t.into())] });
        let p = |t: &str| Out::El(OutEl { tag: "p", attrs: vec![], kids: vec![Out::Text(t.into())] });
        let mut kids = vec![Out::Text(" ".into()), p("a"), Out::Text(" ".into()), p("b"), Out::Text(" ".into())];
        tidy(&mut kids);
        assert_eq!(kids, vec![p("a"), p("b")]);
        let mut inline = vec![Out::Text(" Он ".into()), em("сказал"), Out::Text(" ".into()), em("тихо"), Out::Text(". ".into())];
        tidy(&mut inline);
        assert_eq!(inline, vec![Out::Text("Он ".into()), em("сказал"), Out::Text(" ".into()), em("тихо"), Out::Text(".".into())]);
    }

    #[test]
    fn json_shape() {
        let tree = vec![Out::El(OutEl::new("p").with("id", "a1").with("style", "font-style:italic")), Out::Text("«текст»".into())];
        let mut p = tree;
        if let Out::El(e) = &mut p[0] {
            e.kids.push(Out::Text("x".into()));
        }
        assert_eq!(to_json(&p), r#"[["p",{"id":"a1","style":"font-style:italic"},"x"],"«текст»"]"#);
    }
}

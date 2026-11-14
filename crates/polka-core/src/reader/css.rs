//! Стили книги — только то, что меняет смысл текста.
//!
//! Берём курсив, жирный, капитель, выравнивание, отступы (стихи, эпиграфы), размер шрифта
//! относительно основного и разрывы страниц. Цвета, шрифты, межстрочный интервал и поля задаёт
//! читатель. Значения не копируются, а собираются заново из разрешённых слов и чисел —
//! `url()`, `expression()` и прочее не пройдут. Селекторы — простые: `.класс`, `тег.класс`,
//! `тег`; у составных берётся последняя часть, псевдоклассы и атрибуты пропускаются.

const MAX_CSS: usize = 1 << 20;
const MAX_RULES: usize = 5000;

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    classes: Vec<String>,
    style: Style,
}

/// Готовые объявления: свойство и значение.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    decls: Vec<(&'static str, String)>,
}

impl Style {
    pub fn get(&self, prop: &str) -> Option<&str> {
        self.decls.iter().find(|(p, _)| *p == prop).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, prop: &'static str, value: impl Into<String>) {
        let value = value.into();
        match self.decls.iter_mut().find(|(p, _)| *p == prop) {
            Some(slot) => slot.1 = value,
            None => self.decls.push((prop, value)),
        }
    }

    fn merge(&mut self, other: &Style) {
        for (p, v) in &other.decls {
            self.set(p, v.clone());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }

    /// `display: none` — элемент не показываем вовсе.
    pub fn hidden(&self) -> bool {
        self.get("display") == Some("none")
    }

    pub fn is_pre(&self) -> bool {
        matches!(self.get("white-space"), Some("pre" | "pre-wrap"))
    }

    /// Разрыв страницы до или после — пустой элемент с ним не выбрасываем.
    pub fn breaks(&self) -> bool {
        self.get("break-before").is_some() || self.get("break-after").is_some()
    }

    /// Атрибут `style` у элемента книги.
    pub fn inline(&mut self, css: &str) {
        if css.len() <= 4096 {
            declarations(css, true, self);
        }
    }

    /// Значение атрибута `style` для страницы.
    pub fn to_attr(&self) -> Option<String> {
        let mut parts: Vec<String> = self.decls.iter().filter(|(p, v)| !(*p == "display" && v == "none")).map(|(p, v)| format!("{p}:{v}")).collect();
        // Строка по центру или справа не сдвигается красной строкой.
        if matches!(self.get("text-align"), Some("center" | "right" | "end")) && self.get("text-indent").is_none() {
            parts.push("text-indent:0".into());
        }
        (!parts.is_empty()).then(|| parts.join(";"))
    }
}

impl Sheet {
    pub fn parse(css: &str) -> Sheet {
        if css.len() > MAX_CSS {
            return Sheet::default();
        }
        let css = strip_comments(css);
        let b = css.as_bytes();
        let mut rules = Vec::new();
        let mut i = 0;
        while i < b.len() && rules.len() < MAX_RULES {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b';' || b[i] == b'}') {
                i += 1;
            }
            if i >= b.len() {
                break;
            }
            let brace = css[i..].find('{').map(|p| p + i);
            if b[i] == b'@' {
                // @charset, @import — до точки с запятой; @media, @font-face, @page — блок целиком.
                let semi = css[i..].find(';').map(|p| p + i);
                i = match (semi, brace) {
                    (Some(s), Some(br)) if s < br => s + 1,
                    (_, Some(br)) => block_end(b, br),
                    (Some(s), None) => s + 1,
                    (None, None) => break,
                };
                continue;
            }
            let Some(br) = brace else { break };
            let end = block_end(b, br);
            let body_end = if b.get(end.wrapping_sub(1)) == Some(&b'}') && end - 1 > br { end - 1 } else { end };
            let selectors = &css[i..br];
            let body = &css[br + 1..body_end];
            i = end;
            for sel in selectors.split(',') {
                if let Some((tag, classes)) = selector(sel) {
                    let mut style = Style::default();
                    declarations(body, !classes.is_empty(), &mut style);
                    if !style.is_empty() {
                        rules.push(Rule { tag, classes, style });
                    }
                }
            }
        }
        Sheet { rules }
    }
}

/// Стиль элемента по таблицам стилей главы: сначала правила по тегу, потом по классам —
/// у классов вес больше; внутри — по порядку в файлах.
pub fn style_of(sheets: &[&Sheet], tag: &str, classes: &[&str]) -> Style {
    let mut s = Style::default();
    for sheet in sheets {
        for r in sheet.rules.iter().filter(|r| r.classes.is_empty()) {
            if r.tag.as_deref() == Some(tag) {
                s.merge(&r.style);
            }
        }
    }
    if classes.is_empty() {
        return s;
    }
    for sheet in sheets {
        for r in sheet.rules.iter().filter(|r| !r.classes.is_empty()) {
            if r.tag.as_deref().is_some_and(|t| t != tag) {
                continue;
            }
            if r.classes.iter().all(|c| classes.contains(&c.as_str())) {
                s.merge(&r.style);
            }
        }
    }
    s
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(p) = rest.find("/*") {
        out.push_str(&rest[..p]);
        match rest[p + 2..].find("*/") {
            Some(e) => rest = &rest[p + 2 + e + 2..],
            None => return out,
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// Позиция сразу за парной закрывающей скобкой.
fn block_end(b: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    for (j, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
    }
    b.len()
}

/// Последняя часть селектора: тег и классы. Только тег — лишь для простого селектора (`h1`),
/// иначе правило вроде `div p` легло бы на все абзацы книги.
fn selector(sel: &str) -> Option<(Option<String>, Vec<String>)> {
    let sel = sel.trim();
    if sel.is_empty() || sel.contains([':', '[', '#', '*', '@', '\\']) {
        return None;
    }
    let parts: Vec<&str> = sel.split(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~')).filter(|p| !p.is_empty()).collect();
    let last = *parts.last()?;
    let mut it = last.split('.');
    let tag = it.next().unwrap_or("").to_ascii_lowercase();
    let classes: Vec<String> = it.filter(|c| !c.is_empty()).map(str::to_string).collect();
    let tag = match tag.as_str() {
        "" => None,
        t if t.bytes().all(|b| b.is_ascii_alphanumeric()) => Some(tag),
        _ => return None,
    };
    if classes.is_empty() && (tag.is_none() || parts.len() > 1) {
        return None;
    }
    Some((tag, classes))
}

/// Объявления блока. `full` — правило с классом или атрибут `style`; у правил по одному тегу
/// берутся только начертание и выравнивание (размеры и отступы задаёт читалка).
fn declarations(body: &str, full: bool, out: &mut Style) {
    for d in body.split(';') {
        let Some((prop, value)) = d.split_once(':') else { continue };
        let prop = prop.trim().to_ascii_lowercase();
        let mut value = value.trim().to_ascii_lowercase();
        if let Some(p) = value.find('!') {
            value.truncate(p);
        }
        decl(&prop, value.trim(), full, out);
    }
}

fn decl(prop: &str, v: &str, full: bool, out: &mut Style) {
    match prop {
        "font-style" => match v {
            "italic" | "oblique" => out.set("font-style", "italic"),
            "normal" => out.set("font-style", "normal"),
            _ => {}
        },
        "font-weight" => match v {
            "bold" | "bolder" | "600" | "700" | "800" | "900" => out.set("font-weight", "bold"),
            "normal" | "lighter" | "100" | "200" | "300" | "400" | "500" => out.set("font-weight", "normal"),
            _ => {}
        },
        "font-variant" | "font-variant-caps" => match v {
            "small-caps" | "all-small-caps" => out.set("font-variant", "small-caps"),
            "normal" => out.set("font-variant", "normal"),
            _ => {}
        },
        "text-align" => {
            if matches!(v, "left" | "right" | "center" | "justify" | "start" | "end") {
                out.set("text-align", v);
            }
        }
        "text-decoration" | "text-decoration-line" => {
            let words: Vec<&str> = v.split_whitespace().filter(|w| matches!(*w, "underline" | "line-through" | "overline")).collect();
            if !words.is_empty() {
                out.set("text-decoration", words.join(" "));
            } else if v.split_whitespace().any(|w| w == "none") {
                out.set("text-decoration", "none");
            }
        }
        "vertical-align" => {
            if matches!(v, "super" | "sub" | "baseline" | "middle" | "top" | "bottom" | "text-top" | "text-bottom") {
                out.set("vertical-align", v);
            }
        }
        "text-transform" => {
            if matches!(v, "uppercase" | "lowercase" | "capitalize" | "none") {
                out.set("text-transform", v);
            }
        }
        "display" => {
            if matches!(v, "none" | "block" | "inline" | "inline-block") {
                out.set("display", v);
            }
        }
        "white-space" => {
            if matches!(v, "pre" | "pre-wrap" | "pre-line" | "normal" | "nowrap") {
                out.set("white-space", v);
            }
        }
        "page-break-before" | "break-before" | "page-break-after" | "break-after" => {
            if matches!(v, "always" | "page" | "left" | "right" | "recto" | "verso" | "column") {
                out.set(if prop.ends_with("before") { "break-before" } else { "break-after" }, "column");
            }
        }
        "page-break-inside" | "break-inside" => {
            if v.starts_with("avoid") {
                out.set("break-inside", "avoid");
            }
        }
        _ if !full => {}
        "font-size" => {
            if let Some(size) = font_size(v) {
                out.set("font-size", size);
            }
        }
        "text-indent" => {
            if let Some(len) = length(v, (-4.0, 6.0), Some((-20.0, 30.0))) {
                out.set("text-indent", len);
            }
        }
        "margin-left" | "padding-left" => {
            if let Some(len) = length(v, (0.0, 10.0), Some((0.0, 45.0))) {
                out.set("margin-left", len);
            }
        }
        "margin-right" | "padding-right" => {
            if let Some(len) = length(v, (0.0, 10.0), Some((0.0, 45.0))) {
                out.set("margin-right", len);
            }
        }
        "margin-top" | "padding-top" => {
            if let Some(len) = length(v, (0.0, 4.0), None) {
                out.set("margin-top", len);
            }
        }
        "margin-bottom" | "padding-bottom" => {
            if let Some(len) = length(v, (0.0, 4.0), None) {
                out.set("margin-bottom", len);
            }
        }
        "margin" | "padding" => {
            let w: Vec<&str> = v.split_whitespace().collect();
            let (t, r, b, l) = match w.as_slice() {
                [a] => (*a, *a, *a, *a),
                [a, b] => (*a, *b, *a, *b),
                [a, b, c] => (*a, *b, *c, *b),
                [a, b, c, d] => (*a, *b, *c, *d),
                _ => return,
            };
            let sides = [("margin-top", t, false), ("margin-right", r, true), ("margin-bottom", b, false), ("margin-left", l, true)];
            for (p, val, side) in sides {
                let len = if side { length(val, (0.0, 10.0), Some((0.0, 45.0))) } else { length(val, (0.0, 4.0), None) };
                if let Some(len) = len {
                    out.set(p, len);
                }
            }
        }
        _ => {}
    }
}

/// Число и единица: «1.5em» → (1.5, "em"), «0» → (0, "").
fn number(v: &str) -> Option<(f32, &str)> {
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let n: f32 = v[..end].parse().ok()?;
    n.is_finite().then_some((n, &v[end..]))
}

fn fmt(x: f32) -> String {
    if x.abs() < 0.005 {
        return "0".into();
    }
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Длина в em (или в процентах ширины), в разумных пределах.
fn length(v: &str, (lo, hi): (f32, f32), pct: Option<(f32, f32)>) -> Option<String> {
    let (n, unit) = number(v)?;
    let em = match unit {
        "em" | "rem" | "pc" => n,
        "ex" => n / 2.0,
        "px" => n / 16.0,
        "pt" => n / 12.0,
        "cm" => n * 37.8 / 16.0,
        "mm" => n * 3.78 / 16.0,
        "in" => n * 6.0,
        "%" => {
            let (plo, phi) = pct?;
            let p = n.clamp(plo, phi);
            return Some(if p.abs() < 0.005 { "0".into() } else { format!("{}%", fmt(p)) });
        }
        "" if n == 0.0 => 0.0,
        _ => return None,
    };
    let em = em.clamp(lo, hi);
    Some(if em.abs() < 0.005 { "0".into() } else { format!("{}em", fmt(em)) })
}

fn font_size(v: &str) -> Option<String> {
    let em = match v {
        "xx-small" => 0.6,
        "x-small" => 0.75,
        "small" => 0.89,
        "medium" => 1.0,
        "large" => 1.2,
        "x-large" => 1.5,
        "xx-large" | "xxx-large" => 2.0,
        "smaller" => 0.83,
        "larger" => 1.2,
        _ => {
            let (n, unit) = number(v)?;
            match unit {
                "em" | "rem" => n,
                "%" => n / 100.0,
                "px" => n / 16.0,
                "pt" => n / 12.0,
                "ex" => n / 2.0,
                _ => return None,
            }
        }
    };
    Some(format!("{}em", fmt(em.clamp(0.6, 2.5))))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSS: &str = r#"
        @charset "utf-8";
        @import url("other.css");
        @font-face { font-family: X; src: url(x.ttf) }
        @media amzn-kf8 { .calibre1 { color: red; font-weight: bold } }
        /* комментарий { не правило } */
        .calibre1 { font-style: italic; color: #333 }
        .calibre2 { font-weight: BOLD !important; text-align: center }
        p.epigraph, .poem .line { margin-left: 30%; font-size: 0.9em; }
        h1 { text-align: center; font-size: 2em; page-break-before: always }
        div p { margin-left: 5em }
        a:hover, #main, [lang] { font-weight: bold }
        .hidden { display: none }
        .evil { background: url(javascript:alert(1)); font-family: x; margin-left: expression(alert(1)); text-indent: url(x) }
        .big { font-size: 400% } .neg { text-indent: -50em; margin: 0 auto 1em 12pt }
        p.calibre1.calibre2 { text-decoration: underline line-through }
    "#;

    fn attr(s: &Sheet, tag: &str, classes: &[&str]) -> Option<String> {
        style_of(&[s], tag, classes).to_attr()
    }

    #[test]
    fn picks_meaningful_properties() {
        let s = Sheet::parse(CSS);
        assert_eq!(attr(&s, "span", &["calibre1"]).as_deref(), Some("font-style:italic"));
        assert_eq!(attr(&s, "p", &["calibre2"]).as_deref(), Some("font-weight:bold;text-align:center;text-indent:0"));
        assert_eq!(attr(&s, "p", &["epigraph"]).as_deref(), Some("margin-left:30%;font-size:0.9em"));
        assert_eq!(attr(&s, "div", &["epigraph"]), None);
        assert_eq!(attr(&s, "span", &["line", "x"]).as_deref(), Some("margin-left:30%;font-size:0.9em"));
        // У правила по тегу размер шрифта не берётся, разрыв страницы — берётся.
        assert_eq!(attr(&s, "h1", &[]).as_deref(), Some("text-align:center;break-before:column;text-indent:0"));
        assert_eq!(attr(&s, "p", &[]), None);
        assert!(style_of(&[&s], "p", &["hidden"]).hidden());
        assert_eq!(attr(&s, "p", &["evil"]), None);
        assert_eq!(attr(&s, "p", &["big"]).as_deref(), Some("font-size:2.5em"));
        assert_eq!(attr(&s, "p", &["neg"]).as_deref(), Some("text-indent:-4em;margin-top:0;margin-bottom:1em;margin-left:1em"));
        assert_eq!(
            attr(&s, "p", &["calibre1", "calibre2"]).as_deref(),
            Some("font-style:italic;font-weight:bold;text-align:center;text-decoration:underline line-through;text-indent:0")
        );
    }

    #[test]
    fn inline_style_wins_and_is_filtered() {
        let s = Sheet::parse(".a { font-style: italic }");
        let mut st = style_of(&[&s], "span", &["a"]);
        st.inline("font-style: normal; color: red; text-align: right; font-size: 12pt; behavior: url(x.htc)");
        assert_eq!(st.to_attr().as_deref(), Some("font-style:normal;text-align:right;font-size:1em;text-indent:0"));
    }

    #[test]
    fn broken_css_does_not_panic() {
        for css in ["p {", "}}}", "@media {", ".a { font-style: italic", "p { margin: }", "@", "{}", ".x{font-size:1e40em}", "/* x"] {
            let s = Sheet::parse(css);
            let _ = style_of(&[&s], "p", &["a", "x"]).to_attr();
        }
        let s = Sheet::parse(".a { font-style: italic");
        assert_eq!(attr(&s, "p", &["a"]).as_deref(), Some("font-style:italic"));
    }
}

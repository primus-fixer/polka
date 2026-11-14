//! Глава EPUB (XHTML) → дерево для страницы.
//!
//! Остаются только разрешённые теги; неизвестные разворачиваются (текст сохраняется),
//! опасные (`script`, `iframe`, формы) выбрасываются вместе с содержимым. Ссылки и картинки
//! переводятся в пути внутри книги, стили — через `css`.

use super::css::{style_of, Sheet, Style};
use super::markup::{El, Node};
use super::tree::{is_block, push_text, tidy, Out, OutEl};
use std::cell::RefCell;
use std::collections::HashMap;

/// Куда ведёт ссылка в книге.
pub enum Link {
    /// Глава и метка: `3` или `3#n12`.
    To(String),
    /// Внешний адрес: http, https, mailto.
    Ext(String),
    Nothing,
}

pub struct Ctx<'a> {
    pub sheets: Vec<&'a Sheet>,
    pub link: &'a dyn Fn(&str) -> Link,
    pub image: &'a dyn Fn(&str) -> Option<String>,
    cache: RefCell<HashMap<(String, String), Style>>,
}

impl<'a> Ctx<'a> {
    pub fn new(sheets: Vec<&'a Sheet>, link: &'a dyn Fn(&str) -> Link, image: &'a dyn Fn(&str) -> Option<String>) -> Ctx<'a> {
        Ctx { sheets, link, image, cache: RefCell::new(HashMap::new()) }
    }

    fn style(&self, e: &El) -> Style {
        let classes = e.attr("class").unwrap_or("");
        let key = (e.name.clone(), classes.to_string());
        let mut style = {
            let mut cache = self.cache.borrow_mut();
            cache
                .entry(key)
                .or_insert_with(|| {
                    let list: Vec<&str> = classes.split_whitespace().collect();
                    style_of(&self.sheets, &e.name, &list)
                })
                .clone()
        };
        if let Some(inline) = e.attr("style") {
            style.inline(inline);
        }
        style
    }
}

/// Выбрасываются вместе с содержимым.
const DROP: [&str; 31] = [
    "head", "script", "style", "title", "meta", "link", "noscript", "template", "iframe", "object", "embed", "applet", "audio", "video",
    "canvas", "form", "input", "button", "select", "textarea", "option", "math", "map", "area", "frame", "frameset", "base", "param",
    "source", "track", "xmp",
];

fn map(name: &str) -> Option<&'static str> {
    Some(match name {
        "p" => "p",
        "div" => "div",
        "span" => "span",
        "h1" => "h1",
        "h2" => "h2",
        "h3" => "h3",
        "h4" => "h4",
        "h5" => "h5",
        "h6" => "h6",
        "blockquote" => "blockquote",
        "pre" => "pre",
        "code" | "kbd" | "samp" | "tt" => "code",
        "em" | "var" | "dfn" => "em",
        "i" => "i",
        "strong" => "strong",
        "b" => "b",
        "u" | "ins" => "u",
        "s" | "strike" | "del" => "s",
        "sub" => "sub",
        "sup" => "sup",
        "small" => "small",
        "ul" => "ul",
        "ol" => "ol",
        "li" => "li",
        "dl" => "dl",
        "dt" => "dt",
        "dd" => "dd",
        "table" => "table",
        "thead" => "thead",
        "tbody" => "tbody",
        "tfoot" => "tfoot",
        "tr" => "tr",
        "th" => "th",
        "td" => "td",
        "caption" => "caption",
        "figure" => "figure",
        "figcaption" => "figcaption",
        "section" | "article" => "section",
        "aside" => "aside",
        "header" | "footer" | "nav" | "main" | "center" | "address" | "details" | "summary" | "hgroup" | "body" => "div",
        "cite" => "cite",
        "q" => "q",
        "abbr" | "acronym" => "abbr",
        "mark" => "mark",
        "ruby" => "ruby",
        "rt" => "rt",
        "rp" => "rp",
        "a" => "a",
        "big" | "font" | "label" | "bdi" | "bdo" | "time" | "rb" | "data" | "nobr" => "span",
        _ => return None,
    })
}

/// Метка подходит странице: не пустая, без пробелов и управляющих знаков, не длинная.
pub fn ok_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub fn convert(body: &El, cx: &Ctx) -> Vec<Out> {
    let mut out = Vec::new();
    kids(body, cx, &mut out, false);
    tidy(&mut out);
    out
}

fn kids(el: &El, cx: &Ctx, out: &mut Vec<Out>, pre: bool) {
    for k in &el.kids {
        match k {
            Node::Text(t) => push_text(out, t, pre),
            Node::El(e) => element(e, cx, out, pre),
        }
    }
}

fn image(e: &El, cx: &Ctx) -> Option<Out> {
    let src = e.attr("src").or_else(|| e.attr("href"))?;
    let path = (cx.image)(src)?;
    let mut img = OutEl::new("img").with("src", path);
    if let Some(alt) = e.attr("alt").map(super::squash).filter(|a| !a.is_empty()) {
        img.set("alt", super::clip(&alt, 200));
    }
    Some(Out::El(img))
}

fn element(e: &El, cx: &Ctx, out: &mut Vec<Out>, pre: bool) {
    let name = e.name.as_str();
    if DROP.contains(&name) || e.attr("hidden").is_some() {
        return;
    }
    match name {
        "br" => return out.push(Out::El(OutEl::new("br"))),
        "hr" => return out.push(Out::El(OutEl::new("hr"))),
        "wbr" => return,
        "img" | "image" => {
            if let Some(img) = image(e, cx) {
                out.push(img);
            }
            return;
        }
        // Обложка в EPUB часто — <svg><image xlink:href="…"/></svg>: берём картинку.
        "svg" => {
            let mut images = Vec::new();
            e.find_all("image", &mut images);
            if let Some(img) = images.into_iter().find_map(|i| image(i, cx)) {
                out.push(img);
            }
            return;
        }
        _ => {}
    }
    let mut style = cx.style(e);
    if style.hidden() {
        return;
    }
    let Some(tag) = map(name) else {
        kids(e, cx, out, pre);
        return;
    };
    if name == "center" && style.get("text-align").is_none() {
        style.set("text-align", "center");
    }
    if name == "big" && style.get("font-size").is_none() {
        style.set("font-size", "1.2em");
    }
    let pre = pre || tag == "pre" || style.is_pre();
    let mut el = OutEl::new(tag);
    if let Some(id) = e.attr("id").filter(|id| ok_id(id)) {
        el.set("id", id);
    }
    if tag == "a" {
        if let Some(href) = e.attr("href") {
            match (cx.link)(href) {
                Link::To(to) => el.set("to", to),
                Link::Ext(url) => el.set("ext", url),
                Link::Nothing => {}
            }
        }
        let kind = e.attr("type").unwrap_or("");
        if kind.split_whitespace().any(|w| w == "noteref" || w == "note") || e.attr("role") == Some("doc-noteref") {
            el.set("note", "1");
        }
    }
    if matches!(tag, "td" | "th") {
        for key in ["colspan", "rowspan"] {
            if let Some(n) = e.attr(key).and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| (2..=50).contains(n)) {
                el.set(key, n.to_string());
            }
        }
    }
    if tag == "ol" {
        if let Some(n) = e.attr("start").and_then(|v| v.trim().parse::<i32>().ok()).filter(|n| n.abs() < 100_000) {
            el.set("start", n.to_string());
        }
    }
    if let Some(s) = style.to_attr() {
        el.set("style", s);
    }
    kids(e, cx, &mut el.kids, pre);
    if is_block(tag) && !pre {
        tidy(&mut el.kids);
    }
    let structural = matches!(tag, "td" | "th" | "tr" | "li" | "dd" | "dt");
    if el.kids.is_empty() && el.get("id").is_none() && !style.breaks() && !structural {
        return;
    }
    out.push(Out::El(el));
}

#[cfg(test)]
mod tests {
    use super::super::markup::parse;
    use super::super::tree::to_json;
    use super::*;

    fn run(src: &str, css: &str) -> String {
        let doc = parse(src, true);
        let sheet = Sheet::parse(css);
        let link = |href: &str| -> Link {
            if href.starts_with("http") {
                Link::Ext(href.into())
            } else if let Some(f) = href.strip_prefix("ch2.xhtml#") {
                Link::To(format!("1#{f}"))
            } else {
                Link::Nothing
            }
        };
        let image = |src: &str| -> Option<String> { (src != "missing.png").then(|| format!("OEBPS/{src}")) };
        let cx = Ctx::new(vec![&sheet], &link, &image);
        to_json(&convert(doc.find("body").unwrap(), &cx))
    }

    #[test]
    fn keeps_text_and_safe_tags_only() {
        let src = r#"<html><head><title>x</title><script>alert(1)</script></head><body>
            <h1 class="c1" onclick="evil()">Глава <em>первая</em></h1>
            <p style="text-align:center;background:url(x)">Текст <a href="ch2.xhtml#n1" epub:type="noteref">1</a>
               и <a href="https://example.org">сайт</a>, <a href="javascript:alert(1)">плохая</a>.</p>
            <script>alert(2)</script><iframe src="https://evil"></iframe><form><input value="x"/></form>
            <custom-tag>Свой тег <font color="red">цвет</font></custom-tag>
            <p class="hide">скрыто</p><p hidden="">тоже</p><p>   </p>
            <img src="pic.png" alt="Рисунок" onerror="x()"/><img src="missing.png"/>
            <svg><image xlink:href="cover.jpg"/><script>x</script></svg>
            <div class="pb"></div>
        </body></html>"#;
        let json = run(src, ".c1 { font-style: italic } .hide { display: none } .pb { page-break-before: always }");
        assert_eq!(
            json,
            concat!(
                r#"[["h1",{"style":"font-style:italic"},"Глава ",["em",{},"первая"]],"#,
                r#"["p",{"style":"text-align:center;text-indent:0"},"Текст ",["a",{"to":"1#n1","note":"1"},"1"]," и ",["a",{"ext":"https://example.org"},"сайт"],", ",["a",{},"плохая"],"."],"#,
                r#"" Свой тег ",["span",{},"цвет"]," ","#,
                r#"["img",{"src":"OEBPS/pic.png","alt":"Рисунок"}]," ",["img",{"src":"OEBPS/cover.jpg"}],"#,
                r#"["div",{"style":"break-before:column"}]]"#
            )
        );
        assert!(!json.contains("alert") && !json.contains("onclick") && !json.contains("evil") && !json.contains("скрыто") && !json.contains("тоже"));
    }

    #[test]
    fn preformatted_and_tables() {
        let src = "<body><pre>a\n  b</pre><table><tr><td colspan=\"2\">x</td><td colspan=\"999\"></td></tr></table><ol start=\"5\"><li>пять</li></ol></body>";
        assert_eq!(
            run(src, ""),
            r#"[["pre",{},"a\n  b"],["table",{},["tr",{},["td",{"colspan":"2"},"x"],["td",{}]]],["ol",{"start":"5"},["li",{},"пять"]]]"#
        );
    }
}

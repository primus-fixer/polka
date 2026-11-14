//! EPUB 2 и 3: описание из OPF, порядок глав из spine, оглавление из nav (EPUB 3) или NCX
//! (EPUB 2), обложка — по одному из признаков, которыми её отмечают издатели.

use super::css::Sheet;
use super::html::{self, Ctx, Link};
use super::markup::{self, decode_xml, El};
use super::tree::{first_heading, Out};
use super::{bad, clip, finish, percent_decode, resolve, scheme, squash, year_of, Book, BookFormat, Meta, Res, Result, TocItem, Zip, MAX_CHAPTERS, MAX_TOC};
use std::collections::HashMap;
use std::rc::Rc;

/// Запись описи (manifest): файл книги.
struct Item {
    id: String,
    path: String,
    media: String,
    props: String,
}

fn has_prop(m: &Item, prop: &str) -> bool {
    m.props.split_whitespace().any(|p| p == prop)
}

/// Документ, который можно читать (а не картинка или стиль).
fn is_doc(m: &Item) -> bool {
    match m.media.as_str() {
        "application/xhtml+xml" | "text/html" | "application/xml" | "text/xml" | "application/x-dtbook+xml" => true,
        "" => {
            let l = m.path.to_ascii_lowercase();
            [".xhtml", ".html", ".htm", ".xml"].iter().any(|e| l.ends_with(e))
        }
        _ => false,
    }
}

fn parse_file(zip: &Zip, path: &str, html: bool) -> Result<El> {
    let data = zip.read(path)?;
    Ok(markup::parse(&decode_xml(&data), html))
}

type ChapterOf<'a> = dyn Fn(&str) -> Option<usize> + 'a;

pub(super) fn open(zip: Zip) -> Result<Book> {
    let missing = || bad("В книге EPUB нет описания (OPF)");
    let container = parse_file(&zip, &zip.lookup("META-INF/container.xml").ok_or_else(missing)?, false)?;
    let opf_href = container.find("rootfile").and_then(|r| r.attr("full-path")).ok_or_else(missing)?;
    let opf_path = zip.lookup(&percent_decode(opf_href.trim().trim_start_matches('/'))).ok_or_else(missing)?;
    let opf = parse_file(&zip, &opf_path, false)?;
    let pkg = opf.find("package").unwrap_or(&opf);
    let items = manifest(pkg, &opf_path, &zip);
    let meta = metadata(pkg);

    let mut paths: Vec<String> = Vec::new();
    if let Some(spine) = pkg.find("spine") {
        for r in spine.children("itemref") {
            if let Some(item) = r.attr("idref").and_then(|id| items.iter().find(|m| m.id == id)) {
                if is_doc(item) && !paths.contains(&item.path) {
                    paths.push(item.path.clone());
                }
            }
        }
    }
    if paths.is_empty() {
        // Без порядка чтения — все документы по порядку описи, кроме оглавления.
        for m in items.iter().filter(|m| is_doc(m) && !has_prop(m, "nav")) {
            if !paths.contains(&m.path) {
                paths.push(m.path.clone());
            }
        }
    }
    paths.truncate(MAX_CHAPTERS);
    if paths.is_empty() {
        return Err(bad("В книге нет текста"));
    }
    let index: HashMap<String, usize> = paths.iter().enumerate().map(|(i, p)| (p.to_lowercase(), i)).collect();
    let chapter_of = |path: &str| index.get(&path.to_lowercase()).copied();
    let mut toc = contents(&zip, pkg, &items, &chapter_of);

    let mut sheets: HashMap<String, Rc<Sheet>> = HashMap::new();
    let mut parts: Vec<(String, Vec<Out>)> = Vec::with_capacity(paths.len());
    let mut headings = Vec::with_capacity(paths.len());
    for path in &paths {
        let nodes = chapter(&zip, path, &chapter_of, &mut sheets);
        headings.push(first_heading(&nodes));
        parts.push((String::new(), nodes));
    }
    for (i, part) in parts.iter_mut().enumerate() {
        part.0 = toc.iter().find(|t| t.chapter == i).map(|t| t.title.clone()).or_else(|| headings[i].clone()).unwrap_or_default();
    }
    if toc.is_empty() {
        toc = headings.iter().enumerate().filter_map(|(i, h)| h.clone().map(|title| TocItem { title, chapter: i, anchor: None, depth: 0 })).collect();
    }
    let cover = cover(&zip, pkg, &items, &opf_path);
    finish(BookFormat::Epub, meta, parts, toc, Res::Epub { zip, cover })
}

fn manifest(pkg: &El, opf_path: &str, zip: &Zip) -> Vec<Item> {
    let Some(m) = pkg.find("manifest") else { return Vec::new() };
    m.children("item")
        .filter_map(|it| {
            let (path, _) = resolve(opf_path, it.attr("href")?)?;
            Some(Item {
                id: it.attr("id").unwrap_or("").to_string(),
                path: zip.lookup(&path).unwrap_or(path),
                media: it.attr("media-type").unwrap_or("").trim().to_ascii_lowercase(),
                props: it.attr("properties").unwrap_or("").to_string(),
            })
        })
        .collect()
}

fn metadata(pkg: &El) -> Meta {
    let Some(md) = pkg.find("metadata") else { return Meta::default() };
    let text = |e: &El| squash(&e.text());
    let some = |s: String| (!s.is_empty()).then_some(s);
    // EPUB 3: роль автора — в уточнении <meta refines="#id" property="role">aut</meta>.
    let roles: HashMap<String, String> = md
        .children("meta")
        .filter(|m| m.attr("property") == Some("role"))
        .filter_map(|m| Some((m.attr("refines")?.trim_start_matches('#').to_string(), text(m))))
        .collect();
    let creators: Vec<(String, Option<String>)> = md
        .children("creator")
        .map(|c| (text(c), c.attr("role").map(str::to_string).or_else(|| c.attr("id").and_then(|id| roles.get(id).cloned()))))
        .filter(|(n, _)| !n.is_empty())
        .collect();
    let mut authors: Vec<String> = creators.iter().filter(|(_, r)| r.as_deref().is_none_or(|r| r == "aut")).map(|(n, _)| n.clone()).collect();
    if authors.is_empty() {
        authors = creators.iter().map(|(n, _)| n.clone()).collect();
    }
    authors.dedup();
    authors.truncate(10);
    let year = md
        .children("date")
        .filter(|d| !matches!(d.attr("event"), Some("modification" | "conversion" | "file-creation")))
        .find_map(|d| year_of(&d.text()));
    let isbn = md.children("identifier").find_map(|i| {
        let v = squash(&i.text());
        let marked = i.attr("scheme").is_some_and(|s| s.eq_ignore_ascii_case("isbn")) || v.to_ascii_lowercase().starts_with("urn:isbn:");
        let bare = !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | ' ' | 'X' | 'x'));
        (marked || bare).then(|| crate::isbn::normalize(v.trim_start_matches(|c: char| !c.is_ascii_digit()))).flatten()
    });
    let about = md.child("description").map(|d| markup::parse(&d.text(), true).block_text()).and_then(some).map(|s| clip(&s, 3000));
    Meta {
        title: md.children("title").map(text).find(|t| !t.is_empty()).map(|t| clip(&t, 300)).unwrap_or_default(),
        authors,
        lang: md.child("language").map(text).and_then(some),
        year,
        publisher: md.child("publisher").map(text).and_then(some),
        isbn,
        about,
        genres: md.children("subject").map(text).filter(|s| !s.is_empty() && s.chars().count() <= 50).take(5).collect(),
    }
}

fn target(base: &str, href: &str, chapter_of: &ChapterOf) -> Option<(usize, Option<String>)> {
    let (path, frag) = resolve(base, href)?;
    Some((chapter_of(&path)?, frag))
}

/// Оглавление: nav из EPUB 3, иначе NCX из EPUB 2.
fn contents(zip: &Zip, pkg: &El, items: &[Item], chapter_of: &ChapterOf) -> Vec<TocItem> {
    if let Some(nav) = items.iter().find(|m| has_prop(m, "nav")) {
        if let Ok(doc) = parse_file(zip, &nav.path, true) {
            let toc = nav_toc(&doc, &nav.path, chapter_of);
            if !toc.is_empty() {
                return toc;
            }
        }
    }
    let ncx = pkg
        .find("spine")
        .and_then(|s| s.attr("toc"))
        .and_then(|id| items.iter().find(|m| m.id == id))
        .or_else(|| items.iter().find(|m| m.media == "application/x-dtbncx+xml"));
    match ncx.map(|n| (parse_file(zip, &n.path, false), &n.path)) {
        Some((Ok(doc), path)) => ncx_toc(&doc, path, chapter_of),
        _ => Vec::new(),
    }
}

fn nav_toc(doc: &El, base: &str, chapter_of: &ChapterOf) -> Vec<TocItem> {
    let mut navs = Vec::new();
    doc.find_all("nav", &mut navs);
    let nav = navs.iter().find(|n| n.attr("type").is_some_and(|t| t.split_whitespace().any(|w| w == "toc"))).or_else(|| navs.first());
    let Some(ol) = nav.and_then(|n| n.find("ol")) else { return Vec::new() };
    let mut out = Vec::new();
    nav_list(ol, 0, base, chapter_of, &mut out);
    out
}

fn nav_list(ol: &El, depth: u8, base: &str, chapter_of: &ChapterOf, out: &mut Vec<TocItem>) {
    if depth > 8 {
        return;
    }
    for li in ol.children("li") {
        if out.len() >= MAX_TOC {
            return;
        }
        if let Some(a) = li.elements().find(|e| e.name == "a" || e.name == "span") {
            let title = squash(&a.text());
            if let Some((chapter, anchor)) = a.attr("href").and_then(|h| target(base, h, chapter_of)) {
                if !title.is_empty() {
                    out.push(TocItem { title: clip(&title, 200), chapter, anchor, depth });
                }
            }
        }
        if let Some(sub) = li.child("ol") {
            nav_list(sub, depth + 1, base, chapter_of, out);
        }
    }
}

fn ncx_toc(doc: &El, base: &str, chapter_of: &ChapterOf) -> Vec<TocItem> {
    let Some(map) = doc.find("navmap") else { return Vec::new() };
    let mut out = Vec::new();
    ncx_points(map, 0, base, chapter_of, &mut out);
    out
}

fn ncx_points(parent: &El, depth: u8, base: &str, chapter_of: &ChapterOf, out: &mut Vec<TocItem>) {
    if depth > 8 {
        return;
    }
    for np in parent.children("navpoint") {
        if out.len() >= MAX_TOC {
            return;
        }
        let title = np.child("navlabel").and_then(|l| l.child("text")).map(|t| squash(&t.text())).unwrap_or_default();
        if let Some((chapter, anchor)) = np.child("content").and_then(|c| c.attr("src")).and_then(|s| target(base, s, chapter_of)) {
            if !title.is_empty() {
                out.push(TocItem { title: clip(&title, 200), chapter, anchor, depth });
            }
        }
        ncx_points(np, depth + 1, base, chapter_of, out);
    }
}

/// Глава: XHTML со стилями из `<link>` и `<style>` → дерево для страницы.
fn chapter(zip: &Zip, path: &str, chapter_of: &ChapterOf, cache: &mut HashMap<String, Rc<Sheet>>) -> Vec<Out> {
    let Ok(doc) = parse_file(zip, path, true) else { return Vec::new() };
    let mut sheets: Vec<Rc<Sheet>> = Vec::new();
    if let Some(head) = doc.find("head") {
        for el in head.elements() {
            match el.name.as_str() {
                "link" if el.attr("rel").is_some_and(|r| r.to_ascii_lowercase().split_whitespace().any(|w| w == "stylesheet")) => {
                    if let Some(css) = el.attr("href").and_then(|h| resolve(path, h)).and_then(|(p, _)| zip.lookup(&p)) {
                        let sheet = cache.entry(css.clone()).or_insert_with(|| Rc::new(zip.read(&css).map(|d| Sheet::parse(&decode_xml(&d))).unwrap_or_default()));
                        sheets.push(sheet.clone());
                    }
                }
                "style" => sheets.push(Rc::new(Sheet::parse(&el.text()))),
                _ => {}
            }
        }
    }
    let link = |href: &str| -> Link {
        let href = href.trim();
        if let Some(s) = scheme(href) {
            return if matches!(s.as_str(), "http" | "https" | "mailto") && href.len() <= 2000 { Link::Ext(href.to_string()) } else { Link::Nothing };
        }
        match target(path, href, chapter_of) {
            Some((ch, Some(anchor))) => Link::To(format!("{ch}#{anchor}")),
            Some((ch, None)) => Link::To(ch.to_string()),
            None => Link::Nothing,
        }
    };
    let image = |src: &str| -> Option<String> { zip.lookup(&resolve(path, src)?.0) };
    let refs: Vec<&Sheet> = sheets.iter().map(|s| s.as_ref()).collect();
    let cx = Ctx::new(refs, &link, &image);
    html::convert(doc.find("body").unwrap_or(&doc), &cx)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Обложка: свойство cover-image (EPUB 3), <meta name="cover"> (EPUB 2), «cover» в имени
/// картинки или первая картинка страницы обложки из <guide>.
fn cover(zip: &Zip, pkg: &El, items: &[Item], opf_path: &str) -> Option<String> {
    let is_image = |m: &&Item| m.media.starts_with("image/");
    if let Some(m) = items.iter().find(|m| has_prop(m, "cover-image")) {
        return Some(m.path.clone());
    }
    let named = pkg
        .find("metadata")
        .and_then(|md| md.children("meta").find(|m| m.attr("name").is_some_and(|n| n.eq_ignore_ascii_case("cover"))))
        .and_then(|m| m.attr("content"));
    if let Some(id) = named {
        if let Some(m) = items.iter().find(|m| m.id == id).or_else(|| items.iter().find(|m| m.path.ends_with(id))).filter(is_image) {
            return Some(m.path.clone());
        }
    }
    if let Some(m) = items.iter().filter(is_image).find(|m| m.id.to_ascii_lowercase().contains("cover") || file_name(&m.path).to_ascii_lowercase().contains("cover")) {
        return Some(m.path.clone());
    }
    let page = pkg
        .find("guide")
        .and_then(|g| g.children("reference").find(|r| r.attr("type").is_some_and(|t| t.eq_ignore_ascii_case("cover"))))
        .and_then(|r| resolve(opf_path, r.attr("href")?))
        .and_then(|(p, _)| zip.lookup(&p))?;
    let doc = parse_file(zip, &page, true).ok()?;
    let mut images = Vec::new();
    doc.find_all("img", &mut images);
    doc.find_all("image", &mut images);
    images.iter().find_map(|i| i.attr("src").or_else(|| i.attr("href"))).and_then(|s| resolve(&page, s)).and_then(|(p, _)| zip.lookup(&p))
}

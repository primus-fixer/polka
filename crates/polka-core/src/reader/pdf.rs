//! PDF: ядро берёт из файла только то, что нужно полке, — название, автора и число страниц.
//! Страницы, оглавление, поиск и выделение — в интерфейсе (pdf.js): у PDF нет текста «потоком»,
//! есть страницы, и рисовать их умеет только браузерный движок.
//!
//! Сведения — из словаря Info (lopdf, без чтения всего файла), если их там нет — из XMP
//! (`dc:title`, `dc:creator`). Разбор идёт в отдельном потоке: испорченный файл не уронит
//! «Полку», а лишь оставит книгу без сведений — тогда название берётся из имени файла.

use super::{clip, markup, squash, Meta};

/// Файл похож на PDF: «%PDF-» в начале (бывает, что перед ним немного мусора).
pub fn looks_like(data: &[u8]) -> bool {
    data[..data.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

/// Что известно о PDF.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub meta: Meta,
    /// Страниц; 0 — не удалось узнать.
    pub pages: u32,
    /// Файл зашифрован (обычно — запрет печати и копирования; читать pdf.js может).
    pub encrypted: bool,
}

/// Сведения о PDF. Никогда не падает: что не прочиталось — остаётся пустым.
pub fn info(data: &[u8]) -> Info {
    let parsed = std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("pdf-info".into())
            .stack_size(16 << 20)
            .spawn_scoped(s, || lopdf::Document::load_metadata_mem(data).ok())
            .ok()
            .and_then(|h| h.join().ok().flatten())
    });
    let mut out = Info::default();
    if let Some(m) = parsed {
        out.pages = m.page_count;
        out.encrypted = m.encrypted;
        out.meta.title = m.title.as_deref().map(clean_title).unwrap_or_default();
        out.meta.authors = m.author.as_deref().map(authors).unwrap_or_default();
        out.meta.about = m.subject.as_deref().map(squash).filter(|s| s.chars().count() >= 3 && !junk(s)).map(|s| clip(&s, 1000));
    }
    if out.meta.title.is_empty() || out.meta.authors.is_empty() {
        if let Some((title, creators)) = xmp(data) {
            if out.meta.title.is_empty() {
                out.meta.title = clean_title(&title);
            }
            if out.meta.authors.is_empty() {
                out.meta.authors = creators.iter().flat_map(|c| authors(c)).collect();
            }
        }
    }
    out.meta.authors.dedup();
    out
}

/// Пустые и служебные значения, которые программы пишут вместо названия и автора.
fn junk(s: &str) -> bool {
    const JUNK: &[&str] = &[
        "untitled", "untitled document", "no title", "document", "без имени", "без названия", "документ", "admin", "administrator", "user", "owner", "author",
        "unknown", "anonymous", "windows user", "пользователь", "администратор", "автор", "неизвестный", "microsoft", "microsoft office user",
    ];
    let low = s.trim().to_lowercase();
    // «Document1», «user2» — тоже служебные; а «1984» — название.
    let stem = low.trim_end_matches(|c: char| c.is_ascii_digit()).trim();
    low.chars().all(|c| !c.is_alphanumeric())
        || JUNK.contains(&low.as_str())
        || JUNK.contains(&stem)
        || low.starts_with("http://")
        || low.starts_with("https://")
}

/// Название без следов программы: «Microsoft Word - Пикник.docx» → «Пикник».
fn clean_title(raw: &str) -> String {
    let mut t = squash(raw);
    for prefix in ["Microsoft Word - ", "Microsoft PowerPoint - ", "Microsoft Excel - "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.to_string();
        }
    }
    // Похоже на путь к файлу — оставляем только имя.
    if t.contains(['\\', '/']) && !t.contains(' ') {
        t = t.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    }
    let low = t.to_lowercase();
    for ext in [".docx", ".doc", ".rtf", ".odt", ".pdf", ".txt", ".indd", ".tex", ".djvu", ".fb2", ".epub"] {
        if low.ends_with(ext) {
            t.truncate(t.len() - ext.len());
            break;
        }
    }
    let t = t.trim().to_string();
    if junk(&t) {
        String::new()
    } else {
        clip(&t, 300)
    }
}

/// Авторы из одной строки: «Иванов И. И.; Петров П. П.», «A. Author and B. Author».
fn authors(raw: &str) -> Vec<String> {
    let s = squash(raw);
    let parts: Vec<&str> = if s.contains(';') {
        s.split(';').collect()
    } else if s.contains(" and ") {
        s.split(" and ").collect()
    } else if s.contains(" и ") {
        s.split(" и ").collect()
    } else if s.matches(',').count() >= 2 || s.split(',').all(|p| p.trim().contains(' ')) {
        // «Фамилия, Имя» — одна запятая между частями одного имени; две и больше — список.
        s.split(',').collect()
    } else {
        vec![s.as_str()]
    };
    parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !junk(p)).map(|p| clip(&p, 120)).take(12).collect()
}

/// XMP: название и авторы из первого пакета, где они есть. Пакеты обычно не сжаты.
fn xmp(data: &[u8]) -> Option<(String, Vec<String>)> {
    const OPEN: &[u8] = b"<x:xmpmeta";
    const CLOSE: &[u8] = b"</x:xmpmeta>";
    const MAX_PACKET: usize = 512 << 10;
    let mut from = 0;
    for _ in 0..16 {
        let start = from + find(&data[from..], OPEN)?;
        let window = &data[start..data.len().min(start + MAX_PACKET)];
        let Some(len) = find(window, CLOSE) else {
            from = start + OPEN.len();
            continue;
        };
        from = start + len + CLOSE.len();
        let Ok(text) = std::str::from_utf8(&window[..len + CLOSE.len()]) else { continue };
        let root = markup::parse(text, false);
        let title = root.find("title").map(first_li).unwrap_or_default();
        let creators: Vec<String> = root
            .find("creator")
            .map(|c| {
                let mut items = Vec::new();
                c.find_all("li", &mut items);
                if items.is_empty() {
                    vec![squash(&c.text())]
                } else {
                    items.iter().map(|li| squash(&li.text())).collect()
                }
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        if !title.is_empty() || !creators.is_empty() {
            return Some((title, creators));
        }
    }
    None
}

/// Значение с языком по умолчанию из `rdf:Alt`, иначе первое.
fn first_li(el: &markup::El) -> String {
    let mut items = Vec::new();
    el.find_all("li", &mut items);
    let pick = items.iter().find(|li| li.attr("lang") == Some("x-default")).or(items.first());
    squash(&pick.map(|li| li.text()).unwrap_or_else(|| el.text()))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_authors_are_cleaned() {
        assert_eq!(clean_title("Microsoft Word - Пикник на обочине.docx"), "Пикник на обочине");
        assert_eq!(clean_title("C:\\Users\\me\\book.pdf"), "book");
        assert_eq!(clean_title("Untitled"), "");
        assert_eq!(clean_title("Document1"), "");
        assert_eq!(clean_title("  Dune  "), "Dune");
        assert_eq!(clean_title("1984"), "1984");
        assert_eq!(clean_title("..."), "");
        assert_eq!(authors("Аркадий Стругацкий; Борис Стругацкий"), ["Аркадий Стругацкий", "Борис Стругацкий"]);
        assert_eq!(authors("Frank Herbert"), ["Frank Herbert"]);
        assert_eq!(authors("Herbert, Frank"), ["Herbert, Frank"]);
        assert_eq!(authors("Admin"), Vec::<String>::new());
        assert_eq!(authors("A. One and B. Two"), ["A. One", "B. Two"]);
    }

    #[test]
    fn info_from_real_files() {
        // Сведения в словаре Info (UTF-16), в потоке объектов, в зашифрованном файле; без сведений.
        let plain = info(include_bytes!("testdata/info.pdf"));
        assert_eq!(plain.meta.title, "Пикник на обочине");
        assert_eq!(plain.meta.authors, ["Аркадий Стругацкий", "Борис Стругацкий"]);
        assert_eq!(plain.meta.about.as_deref(), Some("Повесть о Зоне"));
        assert_eq!(plain.pages, 3);
        assert!(!plain.encrypted);
        let packed = info(include_bytes!("testdata/objstm.pdf"));
        assert_eq!((packed.meta.title.as_str(), packed.pages), ("Пикник на обочине", 3));
        let locked = info(include_bytes!("testdata/encrypted.pdf"));
        assert!(locked.encrypted);
        assert_eq!((locked.meta.title.as_str(), locked.pages), ("Пикник на обочине", 3));
        assert_eq!(locked.meta.authors, ["Аркадий Стругацкий"]);
        let bare = info(include_bytes!("testdata/bare.pdf"));
        assert_eq!((bare.meta.title.as_str(), bare.pages), ("", 2));
        assert!(bare.meta.authors.is_empty());
    }

    #[test]
    fn broken_files_do_not_panic() {
        let whole = include_bytes!("testdata/info.pdf");
        for cut in [10, 100, 700, whole.len() / 2, whole.len() - 30] {
            let _ = info(&whole[..cut]);
        }
        let mut garbage = whole.to_vec();
        for (i, b) in garbage.iter_mut().enumerate().skip(200) {
            if i % 7 == 0 {
                *b = b'[';
            }
        }
        let _ = info(&garbage);
        let deep = [b"%PDF-1.4\n1 0 obj ".as_slice(), &[b'['; 100_000], b"\nendobj\ntrailer << /Root 1 0 R >>\n%%EOF"].concat();
        let _ = info(&deep);
        assert_eq!(info(b"%PDF-").pages, 0);
    }

    #[test]
    fn xmp_fallback() {
        let pdf = r#"%PDF-1.7
1 0 obj << /Type /Metadata /Subtype /XML /Length 400 >> stream
<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="ru">Пикник</rdf:li><rdf:li xml:lang="x-default">Пикник на обочине</rdf:li></rdf:Alt></dc:title>
<dc:creator><rdf:Seq><rdf:li>Аркадий Стругацкий</rdf:li><rdf:li>Борис Стругацкий</rdf:li></rdf:Seq></dc:creator></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>
endstream endobj
"#
        .as_bytes();
        let (title, creators) = xmp(pdf).unwrap();
        assert_eq!(title, "Пикник на обочине");
        assert_eq!(creators, ["Аркадий Стругацкий", "Борис Стругацкий"]);
        // Файл без таблицы ссылок: lopdf его не прочитает, а XMP — найдётся.
        let i = info(pdf);
        assert_eq!(i.meta.title, "Пикник на обочине");
        assert_eq!(i.meta.authors.len(), 2);
        assert!(looks_like(pdf));
        assert!(!looks_like(b"PK\x03\x04"));
    }
}

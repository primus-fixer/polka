//! Книги EPUB и FB2, собранные прямо в тесте: разбор, главы, оглавление, обложка, поиск.

use super::*;
use std::io::Write;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01];
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

pub(crate) fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (i, (name, data)) in files.iter().enumerate() {
        // Первая запись (mimetype) — без сжатия, остальные сжаты, как в настоящих книгах.
        let method = if i == 0 { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        w.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const OPF3: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Пикник на обочине</dc:title>
    <dc:creator id="c1">Аркадий Стругацкий</dc:creator>
    <meta refines="#c1" property="role" scheme="marc:relators">aut</meta>
    <dc:creator id="c2">Борис Стругацкий</dc:creator>
    <dc:creator id="t1">Иван Переводчиков</dc:creator>
    <meta refines="#t1" property="role">trl</meta>
    <dc:language>ru</dc:language>
    <dc:date>1972-01-01</dc:date>
    <dc:identifier id="id">urn:isbn:978-5-17-090630-7</dc:identifier>
    <dc:description>&lt;p&gt;Первый абзац.&lt;/p&gt;&lt;p&gt;Второй.&lt;/p&gt;</dc:description>
    <dc:subject>Фантастика</dc:subject>
    <meta property="dcterms:modified">2020-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="css" href="styles/book.css" media-type="text/css"/>
    <item id="cover" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="pic" href="images/pic%20one.png" media-type="image/png"/>
    <item id="ch1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="text/Ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/><itemref idref="ch2"/><itemref idref="ch1"/><itemref idref="nothing"/></spine>
</package>"##;

const NAV: &str = r#"<html xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Оглавление</title></head><body>
  <nav epub:type="landmarks"><ol><li><a href="text/ch2.xhtml">Не то</a></li></ol></nav>
  <nav epub:type="toc"><h1>Содержание</h1><ol>
    <li><a href="text/ch1.xhtml">Глава
        первая</a><ol><li><a href="text/ch1.xhtml#part2">Часть вторая</a></li></ol></li>
    <li><span>Без ссылки</span><ol><li><a href="text/ch2.xhtml">Примечания</a></li></ol></li>
    <li><a href="images/cover.jpg">Не глава</a></li>
  </ol></nav></body></html>"#;

const CH1: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.1//EN" "http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd">
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Глава</title><link rel="stylesheet" type="text/css" href="../styles/book.css"/>
<style>.u { text-decoration: underline }</style></head>
<body>
  <h1 class="c">Глава первая</h1>
  <p>Ёлка&nbsp;и <span class="it">курсив</span>.<a href="Ch2.xhtml#n1" epub:type="noteref">1</a> <span class="u">черта</span></p>
  <p id="part2"><img src="../images/pic%20one.png" alt="Рисунок"/></p>
  <script>alert('нет')</script>
</body></html>"#;

const CH2: &str = r#"<html><body><aside epub:type="footnote" id="n1"><p>Сноска.</p></aside></body></html>"#;

fn epub3() -> Vec<u8> {
    zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#),
        ("OEBPS/content.opf", OPF3.as_bytes()),
        ("OEBPS/nav.xhtml", NAV.as_bytes()),
        ("OEBPS/styles/book.css", b".it { font-style: italic } .c { text-align: center; color: red }"),
        ("OEBPS/images/cover.jpg", JPEG),
        ("OEBPS/images/pic one.png", PNG),
        ("OEBPS/text/ch1.xhtml", CH1.as_bytes()),
        ("OEBPS/text/Ch2.xhtml", CH2.as_bytes()),
    ])
}

#[test]
fn epub3_meta_toc_chapters_and_images() {
    let book = Book::open(epub3()).unwrap();
    assert_eq!(book.format, BookFormat::Epub);
    let m = &book.meta;
    assert_eq!(m.title, "Пикник на обочине");
    assert_eq!(m.authors, ["Аркадий Стругацкий", "Борис Стругацкий"]);
    assert_eq!((m.lang.as_deref(), m.year.as_deref(), m.isbn.as_deref()), (Some("ru"), Some("1972"), Some("9785170906307")));
    assert_eq!(m.about.as_deref(), Some("Первый абзац.\n\nВторой."));
    assert_eq!(m.genres, ["Фантастика"]);

    // Повтор главы в spine и ссылка на несуществующее — пропущены.
    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Глава первая", "Примечания"]);
    let toc: Vec<(&str, usize, Option<&str>, u8)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.anchor.as_deref(), t.depth)).collect();
    assert_eq!(toc, [("Глава первая", 0, None, 0), ("Часть вторая", 0, Some("part2"), 1), ("Примечания", 1, None, 1)]);

    let ch = book.chapter(0).unwrap();
    assert!(ch.starts_with(r#"[["h1",{"style":"text-align:center;text-indent:0"},"Глава первая"]"#), "{ch}");
    assert!(ch.contains(r#"["span",{"style":"font-style:italic"},"курсив"]"#), "{ch}");
    assert!(ch.contains(r#"["a",{"to":"1#n1","note":"1"},"1"]"#), "{ch}");
    assert!(ch.contains(r#"["span",{"style":"text-decoration:underline"},"черта"]"#), "{ch}");
    assert!(ch.contains(r#"["p",{"id":"part2"},["img",{"src":"OEBPS/images/pic one.png","alt":"Рисунок"}]]"#), "{ch}");
    assert!(!ch.contains("alert") && !ch.contains("color"));
    assert_eq!(book.chapter(1).unwrap(), r#"[["aside",{"id":"n1"},["p",{},"Сноска."]]]"#);
    assert_eq!(book.chapter(2), None);
    assert_eq!(book.chapters[0].size, "Глава перваяЁлка\u{a0}и курсив.1 черта".chars().count());
    assert_eq!(book.total(), book.chapters[0].size + "Сноска.".chars().count());

    assert_eq!(book.cover().map(|(m, d)| (m, d.len())), Some(("image/jpeg", JPEG.len())));
    assert_eq!(book.image("OEBPS/images/pic one.png").map(|(m, _)| m), Some("image/png"));
    assert_eq!(book.image("oebps/IMAGES/pic one.png").map(|(m, _)| m), Some("image/png"));
    assert!(book.image("OEBPS/text/ch1.xhtml").is_none());
    assert!(book.image("../../etc/passwd").is_none());
}

#[test]
fn search_ignores_case_and_yo() {
    let book = Book::open(epub3()).unwrap();
    let hits = book.search("ЕЛКА  и", 10);
    assert_eq!(hits.len(), 1);
    let h = &hits[0];
    assert_eq!((h.chapter, h.offset, h.len, h.found.as_str(), h.before.as_str()), (0, 12, 6, "Ёлка\u{a0}и", "Глава первая "));
    assert_eq!(book.search("сноска", 10).iter().map(|h| (h.chapter, h.offset)).collect::<Vec<_>>(), [(1, 0)]);
    assert!(book.search("я", 10).is_empty(), "слишком короткий запрос");
    assert_eq!(book.search("а", 10).len(), 0);
    assert_eq!(book.search("ва", 1).len(), 1);
    // Абзацы в отрывке не слипаются: «первая» и «Ёлка» — в разных блоках.
    assert_eq!(book.search("ёлка", 10)[0].before, "Глава первая ");
    let h = &book.search("курсив", 10)[0];
    assert_eq!((h.before.as_str(), h.after.as_str()), ("Глава первая Ёлка\u{a0}и ", ".1 черта"));
}

const OPF2: &str = r#"<?xml version="1.0"?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf" xmlns:opf="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Старая книга</dc:title>
    <dc:creator opf:role="aut" opf:file-as="Один, Автор">Автор Один</dc:creator>
    <dc:creator opf:role="ill">Художник</dc:creator>
    <dc:identifier opf:scheme="ISBN">0-306-40615-2</dc:identifier>
    <dc:identifier opf:scheme="uuid">urn:uuid:12345678-1234-1234-1234-123456789012</dc:identifier>
    <dc:date opf:event="modification">2019</dc:date>
    <dc:date opf:event="publication">1990</dc:date>
    <meta name="cover" content="cov"/>
  </metadata>
  <manifest>
    <item id="cov" href="img/c.jpg" media-type="image/jpeg"/>
    <item id="ch" href="a.html"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="ch"/></spine>
</package>"#;

const NCX: &str = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
  <navPoint id="p1" playOrder="1"><navLabel><text>Начало</text></navLabel><content src="a.html"/>
    <navPoint id="p2" playOrder="2"><navLabel><text>Глубже</text></navLabel><content src="a.html#deep"/></navPoint>
  </navPoint></navMap></ncx>"#;

#[test]
fn epub2_with_ncx_meta_cover_and_sloppy_html() {
    let data = zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#),
        ("content.opf", OPF2.as_bytes()),
        ("toc.ncx", NCX.as_bytes()),
        ("img/c.jpg", JPEG),
        ("a.html", "<html><head><style>.b{font-weight:bold}</style></head><body><p class=b>Жирный<p id=deep>Второй&nbsp;абзац<br>строка &copy; 1990</body></html>".as_bytes()),
    ]);
    let book = Book::open(data).unwrap();
    assert_eq!(book.meta.authors, ["Автор Один"]);
    assert_eq!(book.meta.isbn.as_deref(), Some("9780306406157"));
    assert_eq!(book.meta.year.as_deref(), Some("1990"));
    let toc: Vec<(&str, usize, Option<&str>, u8)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.anchor.as_deref(), t.depth)).collect();
    assert_eq!(toc, [("Начало", 0, None, 0), ("Глубже", 0, Some("deep"), 1)]);
    assert_eq!(book.chapter(0).unwrap(), "[[\"p\",{\"style\":\"font-weight:bold\"},\"Жирный\"],[\"p\",{\"id\":\"deep\"},\"Второй\u{a0}абзац\",[\"br\",{}],\"строка © 1990\"]]");
    assert!(book.cover().is_some());
}

fn fb2_xml() -> String {
    r##"<?xml version="1.0" encoding="windows-1251"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0" xmlns:l="http://www.w3.org/1999/xlink">
<description>
  <title-info>
    <genre>sf_social</genre><genre>unknown_x</genre><genre>sf</genre>
    <author><first-name>Аркадий</first-name><middle-name>Натанович</middle-name><last-name>Стругацкий</last-name></author>
    <author><nickname>Аноним</nickname></author>
    <book-title>Пикник на обочине</book-title>
    <annotation><p>Зона.</p><p>Сталкер.</p></annotation>
    <date value="1972-01-01">1972</date>
    <coverpage><image l:href="#cover.jpg"/></coverpage>
    <lang>ru</lang>
  </title-info>
  <publish-info><publisher>АСТ</publisher><year>2015</year><isbn>978-5-17-090630-7</isbn></publish-info>
</description>
<body>
  <image l:href="#cover.jpg"/>
  <title><p>Пикник на обочине</p></title>
  <epigraph><p>Ты должен сделать добро из зла.</p><text-author>Р. П. Уоррен</text-author></epigraph>
  <section id="s1"><title><p>Часть первая</p></title>
    <section id="s1-1"><title><p>1</p><p>Рэдрик Шухарт</p></title>
      <p>Текст <emphasis>главы</emphasis> со сноской<a l:href="#n1" type="note">[1]</a>.</p>
      <empty-line/>
      <section><title><p>Подглава</p></title><p>Глубже.</p></section>
    </section>
    <section id="s1-2"><title><p>2</p></title>
      <poem><stanza><v>Строка один</v><v>Строка два</v></stanza></poem>
      <p>Ссылка на <a l:href="#missing">ничто</a> и на <a l:href="https://example.org">сайт</a>.</p>
    </section>
  </section>
</body>
<body name="notes">
  <title><p>Примечания</p></title>
  <section id="n1"><title><p>1</p></title><p>Сноска.</p></section>
</body>
<binary id="cover.jpg" content-type="image/jpeg">/9j/4AAQ
SkZJRgABAQ==</binary>
</FictionBook>"##
        .to_string()
}

fn fb2_bytes() -> Vec<u8> {
    let xml = fb2_xml();
    let (bytes, _, bad) = encoding_rs::WINDOWS_1251.encode(&xml);
    assert!(!bad);
    bytes.into_owned()
}

#[test]
fn fb2_in_windows_1251() {
    let book = Book::open(fb2_bytes()).unwrap();
    assert_eq!(book.format, BookFormat::Fb2);
    let m = &book.meta;
    assert_eq!(m.title, "Пикник на обочине");
    assert_eq!(m.authors, ["Аркадий Стругацкий", "Аноним"]);
    assert_eq!(m.genres, ["Социальная фантастика", "Фантастика"]);
    assert_eq!((m.year.as_deref(), m.publisher.as_deref(), m.isbn.as_deref()), (Some("1972"), Some("АСТ"), Some("9785170906307")));
    assert_eq!(m.about.as_deref(), Some("Зона.\n\nСталкер."));

    let titles: Vec<&str> = book.chapters.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Пикник на обочине", "Часть первая", "1. Рэдрик Шухарт", "Подглава", "2", "Примечания"]);
    let toc: Vec<(&str, usize, u8)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.depth)).collect();
    assert_eq!(toc, [("Часть первая", 1, 0), ("1. Рэдрик Шухарт", 2, 1), ("Подглава", 3, 2), ("2", 4, 1), ("Примечания", 5, 0)]);

    assert_eq!(
        book.chapter(0).unwrap(),
        r#"[["div",{"class":"image"},["img",{"src":"cover.jpg"}]],["h1",{},"Пикник на обочине"],["blockquote",{"class":"epigraph"},["p",{},"Ты должен сделать добро из зла."],["p",{"class":"author"},"Р. П. Уоррен"]]]"#
    );
    assert_eq!(book.chapter(1).unwrap(), r#"[["section",{"id":"s1"},["h2",{},"Часть первая"]]]"#);
    assert_eq!(
        book.chapter(2).unwrap(),
        r#"[["section",{"id":"s1-1"},["h2",{},"1",["br",{}],"Рэдрик Шухарт"],["p",{},"Текст ",["em",{},"главы"]," со сноской",["a",{"to":"5#n1","note":"1"},"[1]"],"."],["div",{"class":"gap"}]]]"#
    );
    assert_eq!(book.chapter(3).unwrap(), r#"[["section",{},["h2",{},"Подглава"],["p",{},"Глубже."]]]"#);
    assert_eq!(
        book.chapter(4).unwrap(),
        r#"[["section",{"id":"s1-2"},["h2",{},"2"],["div",{"class":"poem"},["div",{"class":"stanza"},["p",{"class":"verse"},"Строка один"],["p",{"class":"verse"},"Строка два"]]],["p",{},"Ссылка на ",["a",{},"ничто"]," и на ",["a",{"ext":"https://example.org"},"сайт"],"."]]]"#
    );
    assert_eq!(book.chapter(5).unwrap(), r#"[["h1",{},"Примечания"],["section",{"id":"n1"},["h4",{},"1"],["p",{},"Сноска."]]]"#);
    assert_eq!(book.cover().map(|(m, _)| m), Some("image/jpeg"));
    assert_eq!(book.image("cover.jpg").map(|(m, _)| m), Some("image/jpeg"));
    assert!(book.image("nothing.png").is_none());
}

#[test]
fn fb2_inside_zip_and_utf8() {
    let zipped = zip_of(&[("Пикник.fb2", &fb2_bytes())]);
    assert_eq!(Book::open(zipped).unwrap().meta.title, "Пикник на обочине");
    let utf8 = fb2_xml().replace("windows-1251", "utf-8");
    assert_eq!(Book::open(utf8.into_bytes()).unwrap().chapters.len(), 6);
}

#[test]
fn deep_fb2_nesting_stays_in_toc_with_own_anchors() {
    let xml = "<FictionBook><body><section><title><p>A</p></title><section><title><p>B</p></title><section><title><p>C</p></title>\
               <section><title><p>D</p></title><p>текст</p></section></section></section></section></body></FictionBook>";
    let book = Book::open(xml.as_bytes().to_vec()).unwrap();
    let toc: Vec<(&str, usize, Option<&str>, u8)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter, t.anchor.as_deref(), t.depth)).collect();
    // A и B — главы (с короткой головой), C — глава уровня 2, D — метка внутри C.
    assert_eq!(toc, [("A", 0, None, 0), ("B", 1, None, 1), ("C", 2, None, 2), ("D", 2, Some("polka-1"), 3)]);
    assert!(book.chapter(2).unwrap().contains(r#"["section",{"id":"polka-1"},["h3",{},"D"]"#));
}

#[test]
fn not_a_book() {
    let err = |data: Vec<u8>| Book::open(data).err().map(|e| e.to_string()).unwrap_or_default();
    assert_eq!(err(b"hello".to_vec()), "Это не книга EPUB, FB2 или PDF");
    assert_eq!(err(b"%PDF-1.7\n".to_vec()), "Это PDF: его страницы показывает читалка PDF");
    assert_eq!(err(zip_of(&[("readme.txt", b"x")])), "В архиве нет книги EPUB или FB2");
    assert_eq!(err(zip_of(&[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", b"<container/>")])), "В книге EPUB нет описания (OPF)");
    let no_text = zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="c.opf"/></rootfiles></container>"#),
        ("c.opf", b"<package><manifest/><spine/></package>"),
    ]);
    assert_eq!(err(no_text), "В книге нет текста");
    assert_eq!(err(b"<FictionBook><body></body></FictionBook>".to_vec()), "В книге нет текста");
    assert_eq!(err(b"PK\x03\x04broken".to_vec()), "Архив книги повреждён");
}

#[test]
fn paths_inside_the_book() {
    let r = |base: &str, href: &str| resolve(base, href);
    assert_eq!(r("OEBPS/text/ch1.xhtml", "../images/a%20b.png#x"), Some(("OEBPS/images/a b.png".into(), Some("x".into()))));
    assert_eq!(r("OEBPS/text/ch1.xhtml", "#n1"), Some(("OEBPS/text/ch1.xhtml".into(), Some("n1".into()))));
    assert_eq!(r("OEBPS/text/ch1.xhtml", "./b.xhtml?x=1"), Some(("OEBPS/text/b.xhtml".into(), None)));
    assert_eq!(r("x/a.xhtml", "/root.css"), Some(("root.css".into(), None)));
    assert_eq!(r("a.xhtml", "../../etc/passwd"), None);
    assert_eq!(r("x/a.xhtml", "https://example.org/a"), None);
    assert_eq!(r("x/a.xhtml", "mailto:a@b.c"), None);
    assert_eq!(r("x/a.xhtml", "//host/x"), None);
    assert_eq!(r("x/a.xhtml", ""), None);
    assert_eq!(scheme("javascript:alert(1)").as_deref(), Some("javascript"));
    assert_eq!(scheme("C:/x"), None);
    assert_eq!(percent_decode("%D0%81%zz%4"), "Ё%zz%4");
}

#[test]
fn reading_record_checks() {
    let ok = Reading { title: "abc".into(), book: "a".repeat(64), chapter: 3, offset: 120, percent: 45.5, at: 1, minutes: 30, years: [("2026".to_string(), 30)].into() };
    assert!(ok.check().is_ok());
    assert!(Reading { book: "xyz".into(), ..ok.clone() }.check().is_err());
    assert!(Reading { percent: 101.0, ..ok.clone() }.check().is_err());
    assert!(Reading { percent: f32::NAN, ..ok.clone() }.check().is_err());
    assert!(Reading { title: String::new(), ..ok.clone() }.check().is_err());
    assert!(Reading { years: [("26".to_string(), 1)].into(), ..ok.clone() }.check().is_err());
    // Старые записи без минут читаются.
    let old: Reading = serde_json::from_str(&format!(r#"{{"title":"a","book":"{}","chapter":0,"offset":0,"percent":0,"at":0}}"#, "b".repeat(64))).unwrap();
    assert_eq!((old.minutes, old.years.len()), (0, 0));
}

#[test]
fn marks_check() {
    let quote = Mark {
        id: "0123456789abcdef".into(),
        title: "t1".into(),
        book: "c".repeat(64),
        kind: MarkKind::Quote,
        chapter: 2,
        offset: 10,
        end: 40,
        text: "Мой свет — простое «не напрасно»".into(),
        note: String::new(),
        percent: 36.5,
        created: 1,
    };
    assert!(quote.check().is_ok());
    assert!(Mark { end: 10, ..quote.clone() }.check().is_err(), "пустая цитата");
    assert!(Mark { text: "  ".into(), ..quote.clone() }.check().is_err());
    assert!(Mark { text: "я".repeat(4001), ..quote.clone() }.check().is_err());
    assert!(Mark { id: "../x".into(), ..quote.clone() }.check().is_err());
    let bookmark = Mark { kind: MarkKind::Bookmark, end: 0, text: String::new(), ..quote.clone() };
    assert!(bookmark.check().is_ok());
    let json = serde_json::to_string(&bookmark).unwrap();
    assert!(json.contains(r#""kind":"bookmark""#) && !json.contains("note") && !json.contains("text"));
    assert_eq!(serde_json::from_str::<Mark>(&json).unwrap(), bookmark);
}

/// Случайная разметка не роняет разбор: срезы строк только по границам знаков.
#[test]
fn random_markup_never_panics() {
    let pieces = ["<", ">", "/", "<p>", "</p>", "<br/>", "&", "&#", "&#x", ";", "&nbsp;", "\"", "'", "=", " ", "ё", "€", "😀", "<!--", "-->",
                  "<![CDATA[", "]]>", "<!DOCTYPE", "[", "]", "<?", "?>", "<style>", "</style", "<a href=", "#x", "<svg><image href=", "a", "\n"];
    let mut seed = 0x2545F4914F6CDD1Du64;
    for _ in 0..3000 {
        let mut s = String::new();
        for _ in 0..(seed % 40) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            s.push_str(pieces[(seed % pieces.len() as u64) as usize]);
        }
        for html in [true, false] {
            let doc = markup::parse(&s, html);
            let sheet = css::Sheet::parse(&s);
            let link = |_: &str| html::Link::Nothing;
            let image = |s: &str| Some(s.to_string());
            let cx = html::Ctx::new(vec![&sheet], &link, &image);
            let _ = tree::to_json(&html::convert(&doc, &cx));
            let _ = doc.block_text();
        }
        let _ = Book::open(s.clone().into_bytes());
        let _ = resolve("a/b.xhtml", &s);
    }
}

/// Скорость: большая книга открывается быстро (запуск: cargo test --release -- --ignored big_book).
#[test]
#[ignore]
fn big_book_opens_fast() {
    let para = "<p>Он шёл по <em>улице</em>, и&nbsp;фонари горели — один за другим, как в старой песне.</p>\n";
    let chapter = |i: usize| format!("<html><head><link rel=\"stylesheet\" href=\"s.css\"/></head><body><h2 class=\"c\">Глава {i}</h2>{}</body></html>", para.repeat(400));
    let chapters: Vec<String> = (0..60).map(chapter).collect();
    let mut manifest = String::new();
    let mut spine = String::new();
    for i in 0..60 {
        manifest.push_str(&format!("<item id=\"c{i}\" href=\"c{i}.xhtml\" media-type=\"application/xhtml+xml\"/>"));
        spine.push_str(&format!("<itemref idref=\"c{i}\"/>"));
    }
    let opf = format!("<package><metadata><dc:title>Большая</dc:title></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>");
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("mimetype".into(), b"application/epub+zip".to_vec()),
        ("META-INF/container.xml".into(), br#"<container><rootfiles><rootfile full-path="o.opf"/></rootfiles></container>"#.to_vec()),
        ("o.opf".into(), opf.into_bytes()),
        ("s.css".into(), b".c { text-align: center }".to_vec()),
    ];
    for (i, c) in chapters.iter().enumerate() {
        files.push((format!("c{i}.xhtml"), c.clone().into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    let data = zip_of(&refs);
    let raw: usize = chapters.iter().map(|c| c.len()).sum();
    let t = std::time::Instant::now();
    let book = Book::open(data).unwrap();
    let took = t.elapsed();
    eprintln!("{} МБ разметки, {} знаков текста: {:?}", raw >> 20, book.total(), took);
    let t = std::time::Instant::now();
    let hits = book.search("фонари", 100);
    eprintln!("поиск: {} мест за {:?}", hits.len(), t.elapsed());
    assert!(took.as_millis() < 3000);
}

/// Книга одним большим файлом делится на части; ссылки и оглавление ведут в нужную часть.
#[test]
fn big_chapter_is_split_with_links_kept() {
    let para = |i: usize| format!("<p id=\"p{i}\">{}</p>", "Слово за словом, строка за строкой. ".repeat(60));
    let body: String = (0..150).map(para).collect();
    let ch1 = format!("<html><body><div class=\"book\" id=\"top\"><h1>Всё сразу</h1>{body}<p>Ссылка <a href=\"#p140\">вниз</a>.</p></div></body></html>");
    let opf = r#"<package><metadata><dc:title>Одним файлом</dc:title></metadata><manifest>
        <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/><item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="a"/><itemref idref="b"/></spine></package>"#;
    let ncx = r#"<ncx><navMap><navPoint><navLabel><text>Начало</text></navLabel><content src="a.xhtml"/></navPoint>
        <navPoint><navLabel><text>Сто сорок</text></navLabel><content src="a.xhtml#p140"/></navPoint>
        <navPoint><navLabel><text>Конец</text></navLabel><content src="b.xhtml"/></navPoint></navMap></ncx>"#;
    let data = zip_of(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="o.opf"/></rootfiles></container>"#),
        ("o.opf", opf.as_bytes()),
        ("toc.ncx", ncx.as_bytes()),
        ("a.xhtml", ch1.as_bytes()),
        ("b.xhtml", r#"<html><body><p>Последняя глава, <a href="a.xhtml#p3">назад</a>.</p></body></html>"#.as_bytes()),
    ]);
    let book = Book::open(data).unwrap();
    let n = book.chapters.len();
    assert!(n >= 4, "глава поделена: {n}");
    assert!(book.chapters.iter().all(|c| c.size <= BIG));
    assert!(book.chapters[..n - 1].iter().all(|c| c.title == "Начало"));
    // Метка обёртки осталась якорем в первой части.
    assert!(book.chapter(0).unwrap().starts_with(r#"[["span",{"id":"top"}],["h1",{},"Всё сразу"]"#));
    // Оглавление: «Сто сорок» — в той части, где абзац p140; «Конец» — последняя глава.
    let toc: Vec<(&str, usize)> = book.toc.iter().map(|t| (t.title.as_str(), t.chapter)).collect();
    let part_of = |id: &str| (0..n).find(|&i| book.chapter(i).unwrap().contains(&format!(r#"{{"id":"{id}"}}"#))).unwrap();
    assert_eq!(toc, [("Начало", 0), ("Сто сорок", part_of("p140")), ("Конец", n - 1)]);
    // Ссылки: вниз — в часть с p140, из последней главы — в часть с p3.
    let last_part = book.chapter(part_of("p140")).unwrap().to_string();
    let link_from = (0..n).find(|&i| book.chapter(i).unwrap().contains("вниз")).unwrap();
    assert!(book.chapter(link_from).unwrap().contains(&format!(r#""to":"{}#p140""#, part_of("p140"))), "{last_part:.0}");
    assert!(book.chapter(n - 1).unwrap().contains(&format!(r#""to":"{}#p3""#, part_of("p3"))));
}

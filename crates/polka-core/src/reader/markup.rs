//! Терпимый разбор XML и XHTML.
//!
//! Книги из сети бывают с ошибками разметки и с именованными сущностями HTML (`&nbsp;`, `&mdash;`),
//! на которых строгий разбор XML останавливается. Здесь разметка разбирается всегда: незакрытые
//! теги закрываются, лишние закрывающие пропускаются, сущности HTML 4 узнаются. Имена тегов и
//! атрибутов — местные и в нижнем регистре: `dc:title` → `title`, `xlink:href` → `href`.
//! Глубина вложенности ограничена: обход дерева рекурсией не переполнит стек.

use std::borrow::Cow;

/// Глубже разметку не вкладываем: лишние вложенные теги пропускаются, текст остаётся.
pub const MAX_DEPTH: usize = 200;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    El(El),
    Text(String),
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Дочерние элементы.
    pub fn elements(&self) -> impl Iterator<Item = &El> {
        self.kids.iter().filter_map(|k| match k {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&El> {
        self.elements().find(|e| e.name == name)
    }

    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.elements().filter(move |e| e.name == name)
    }

    /// Первый потомок с таким именем (в порядке документа), сам элемент не считается.
    pub fn find(&self, name: &str) -> Option<&El> {
        for e in self.elements() {
            if e.name == name {
                return Some(e);
            }
            if let Some(found) = e.find(name) {
                return Some(found);
            }
        }
        None
    }

    /// Все потомки с таким именем, в порядке документа.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            }
            e.find_all(name, out);
        }
    }

    /// Весь текст внутри.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, out: &mut String) {
        for k in &self.kids {
            match k {
                Node::Text(t) => out.push_str(t),
                Node::El(e) => e.collect_text(out),
            }
        }
    }

    /// Текст по абзацам: блоки (`p`, `div`, `br`, `li`, заголовки, строки стихов) разделяются
    /// пустой строкой — для аннотаций.
    pub fn block_text(&self) -> String {
        let mut paras = Vec::new();
        let mut cur = String::new();
        self.blocks(&mut paras, &mut cur);
        flush(&mut paras, &mut cur);
        paras.join("\n\n")
    }

    fn blocks(&self, paras: &mut Vec<String>, cur: &mut String) {
        for k in &self.kids {
            match k {
                Node::Text(t) => cur.push_str(t),
                Node::El(e) if matches!(e.name.as_str(), "script" | "style" | "binary" | "head") => {}
                Node::El(e) => {
                    let block = matches!(
                        e.name.as_str(),
                        "p" | "div" | "br" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "section" | "v" | "subtitle" | "empty-line" | "title" | "tr"
                    );
                    if block {
                        flush(paras, cur);
                    }
                    e.blocks(paras, cur);
                    if block {
                        flush(paras, cur);
                    }
                }
            }
        }
    }
}

fn flush(paras: &mut Vec<String>, cur: &mut String) {
    let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
    if !t.is_empty() {
        paras.push(t);
    }
    cur.clear();
}

/// Разбирает документ; `html` — правила HTML: пустые теги (`<br>`), сырой текст `<style>` и
/// `<script>`, неявное закрытие `<p>` и `<li>`. Возвращает корень `#root`.
pub fn parse(src: &str, html: bool) -> El {
    let mut p = Parser { s: src, b: src.as_bytes(), i: 0, html, stack: vec![El { name: "#root".into(), ..El::default() }] };
    p.run();
    while p.stack.len() > 1 {
        p.close_top();
    }
    p.stack.pop().unwrap_or_default()
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    html: bool,
    stack: Vec<El>,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

/// Местное имя в нижнем регистре.
fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

fn find_from(s: &str, at: usize, pat: &str) -> Option<usize> {
    s.get(at..).and_then(|rest| rest.find(pat)).map(|p| p + at)
}

const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Блочные теги: в HTML открытие такого закрывает незакрытый `<p>`.
const CLOSES_P: [&str; 26] = [
    "p", "div", "ul", "ol", "dl", "table", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "section", "article", "aside",
    "header", "footer", "nav", "figure", "hr", "address", "center", "main", "details",
];

impl Parser<'_> {
    fn run(&mut self) {
        while self.i < self.b.len() {
            let next = find_from(self.s, self.i, "<").unwrap_or(self.b.len());
            if next > self.i {
                let raw = &self.s[self.i..next];
                self.text(&decode(raw));
                self.i = next;
                continue;
            }
            let rest = &self.s[self.i..];
            if rest.starts_with("<!--") {
                self.i = find_from(self.s, self.i + 4, "-->").map(|e| e + 3).unwrap_or(self.b.len());
            } else if rest.starts_with("<![CDATA[") {
                let end = find_from(self.s, self.i + 9, "]]>").unwrap_or(self.b.len());
                let raw = &self.s[self.i + 9..end];
                self.text(raw);
                self.i = (end + 3).min(self.b.len());
            } else if rest.starts_with("<!") {
                self.skip_declaration();
            } else if rest.starts_with("<?") {
                self.i = find_from(self.s, self.i + 2, "?>").map(|e| e + 2).or_else(|| find_from(self.s, self.i, ">").map(|e| e + 1)).unwrap_or(self.b.len());
            } else if rest.starts_with("</") {
                self.end_tag();
            } else if self.b.get(self.i + 1).is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80) {
                self.start_tag();
            } else {
                self.text("<");
                self.i += 1;
            }
        }
    }

    /// `<!DOCTYPE …>`, в том числе с внутренним подмножеством в квадратных скобках.
    fn skip_declaration(&mut self) {
        let mut depth = 0i32;
        let mut j = self.i + 2;
        while j < self.b.len() {
            match self.b[j] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                b'>' if depth <= 0 => {
                    self.i = j + 1;
                    return;
                }
                _ => {}
            }
            j += 1;
        }
        self.i = self.b.len();
    }

    fn text(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        let top = self.stack.last_mut().expect("корень всегда есть");
        if let Some(Node::Text(last)) = top.kids.last_mut() {
            last.push_str(t);
        } else {
            top.kids.push(Node::Text(t.to_string()));
        }
    }

    fn start_tag(&mut self) {
        let len = self.b.len();
        let start = self.i + 1;
        let mut j = start;
        while j < len && !is_ws(self.b[j]) && self.b[j] != b'>' && self.b[j] != b'/' {
            j += 1;
        }
        let name = local(&self.s[start..j]);
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut closed = false;
        loop {
            while j < len && is_ws(self.b[j]) {
                j += 1;
            }
            if j >= len {
                break;
            }
            match self.b[j] {
                b'>' => {
                    j += 1;
                    break;
                }
                b'/' => {
                    j += 1;
                    if self.b.get(j) == Some(&b'>') {
                        closed = true;
                        j += 1;
                        break;
                    }
                }
                _ => {
                    let ns = j;
                    while j < len && !is_ws(self.b[j]) && self.b[j] != b'=' && self.b[j] != b'>' && self.b[j] != b'/' {
                        j += 1;
                    }
                    let attr = &self.s[ns..j];
                    while j < len && is_ws(self.b[j]) {
                        j += 1;
                    }
                    let mut value = String::new();
                    if j < len && self.b[j] == b'=' {
                        j += 1;
                        while j < len && is_ws(self.b[j]) {
                            j += 1;
                        }
                        if j < len && (self.b[j] == b'"' || self.b[j] == b'\'') {
                            let q = if self.b[j] == b'"' { "\"" } else { "'" };
                            let vs = j + 1;
                            let ve = find_from(self.s, vs, q).unwrap_or(len);
                            value = decode(&self.s[vs..ve]).into_owned();
                            j = (ve + 1).min(len);
                        } else {
                            let vs = j;
                            while j < len && !is_ws(self.b[j]) && self.b[j] != b'>' {
                                j += 1;
                            }
                            value = decode(&self.s[vs..j]).into_owned();
                        }
                    }
                    if !attr.is_empty() {
                        let key = local(attr);
                        if !attrs.iter().any(|(k, _)| *k == key) {
                            attrs.push((key, value));
                        }
                    }
                }
            }
        }
        self.i = j;
        self.open(name, attrs, closed);
    }

    fn open(&mut self, name: String, attrs: Vec<(String, String)>, mut closed: bool) {
        if self.html {
            self.implicit_close(&name);
            if VOID.contains(&name.as_str()) {
                closed = true;
            }
            if !closed && matches!(name.as_str(), "script" | "style" | "textarea" | "title" | "xmp") {
                let end = self.raw_end(&name);
                let raw = &self.s[self.i..end];
                let text = if name == "title" { decode(raw).into_owned() } else { raw.to_string() };
                self.i = find_from(self.s, end, ">").map(|e| e + 1).unwrap_or(self.b.len());
                let kids = if text.is_empty() { Vec::new() } else { vec![Node::Text(text)] };
                self.append(El { name, attrs, kids });
                return;
            }
        }
        let el = El { name, attrs, kids: Vec::new() };
        if closed {
            self.append(el);
        } else if self.stack.len() <= MAX_DEPTH {
            self.stack.push(el);
        }
    }

    /// Где кончается сырой текст `<style>`: перед `</style` без учёта регистра.
    fn raw_end(&self, name: &str) -> usize {
        let mut at = self.i;
        while let Some(p) = find_from(self.s, at, "</") {
            let tail = &self.b[p + 2..];
            if tail.len() >= name.len() && tail[..name.len()].eq_ignore_ascii_case(name.as_bytes()) {
                return p;
            }
            at = p + 2;
        }
        self.b.len()
    }

    fn top(&self) -> &str {
        self.stack.last().map(|e| e.name.as_str()).unwrap_or("")
    }

    fn implicit_close(&mut self, name: &str) {
        let block = CLOSES_P.contains(&name) || matches!(name, "li" | "dt" | "dd" | "tr" | "td" | "th");
        if block && self.top() == "p" {
            self.close_top();
        }
        let close = match name {
            "li" => self.top() == "li",
            "dt" | "dd" => matches!(self.top(), "dt" | "dd"),
            "td" | "th" => matches!(self.top(), "td" | "th"),
            "tr" => {
                // <tr> после <td>: закрыть ячейку, а потом и прежнюю строку.
                if matches!(self.top(), "td" | "th") {
                    self.close_top();
                }
                self.top() == "tr"
            }
            _ => false,
        };
        if close {
            self.close_top();
        }
    }

    fn end_tag(&mut self) {
        let len = self.b.len();
        let start = self.i + 2;
        let mut j = start;
        while j < len && !is_ws(self.b[j]) && self.b[j] != b'>' {
            j += 1;
        }
        let name = local(&self.s[start..j]);
        self.i = find_from(self.s, j, ">").map(|e| e + 1).unwrap_or(len);
        if let Some(pos) = self.stack.iter().rposition(|e| e.name == name) {
            if pos == 0 {
                return;
            }
            while self.stack.len() > pos {
                self.close_top();
            }
        } else if self.html && name == "br" {
            self.append(El { name, ..El::default() });
        }
    }

    fn close_top(&mut self) {
        if self.stack.len() < 2 {
            return;
        }
        let el = self.stack.pop().expect("проверено выше");
        self.append(el);
    }

    fn append(&mut self, el: El) {
        self.stack.last_mut().expect("корень всегда есть").kids.push(Node::El(el));
    }
}

/// Текст с заменой сущностей: `&amp;`, `&#171;`, `&#xAB;`, `&laquo;`. Незнакомое остаётся как было.
pub fn decode(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p..];
        let window = &tail.as_bytes()[..tail.len().min(34)];
        if let Some(semi) = window.iter().position(|&b| b == b';') {
            if let Some(c) = entity(&tail[1..semi]) {
                out.push(c);
                rest = &tail[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return Some(numeric(code));
    }
    let code = match name {
        "amp" => 38,
        "lt" => 60,
        "gt" => 62,
        "quot" => 34,
        "apos" => 39,
        "nbsp" => 160,
        "mdash" => 8212,
        "ndash" => 8211,
        "laquo" => 171,
        "raquo" => 187,
        "hellip" => 8230,
        "shy" => 173,
        _ => {
            if let Some(i) = LATIN1.iter().position(|&n| n == name) {
                161 + i as u32
            } else {
                NAMED.iter().find(|(n, _)| *n == name)?.1
            }
        }
    };
    char::from_u32(code)
}

/// Числовая ссылка; коды 128–159 — как в windows-1252 (так их понимают браузеры).
fn numeric(code: u32) -> char {
    const CP1252: [u32; 32] = [
        0x20AC, 0xFFFD, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0xFFFD, 0x017D, 0xFFFD,
        0xFFFD, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0xFFFD, 0x017E, 0x0178,
    ];
    let code = if (0x80..0xA0).contains(&code) { CP1252[(code - 0x80) as usize] } else { code };
    match code {
        0 => '\u{FFFD}',
        _ => char::from_u32(code).unwrap_or('\u{FFFD}'),
    }
}

/// Сущности Latin-1 по порядку: `&iexcl;` — 161, …, `&yuml;` — 255.
const LATIN1: &[&str] = &[
    "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf", "laquo", "not", "shy", "reg", "macr", "deg",
    "plusmn", "sup2", "sup3", "acute", "micro", "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34",
    "iquest", "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute", "Ecirc", "Euml", "Igrave",
    "Iacute", "Icirc", "Iuml", "ETH", "Ntilde", "Ograve", "Oacute", "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute",
    "Ucirc", "Uuml", "Yacute", "THORN", "szlig", "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil", "egrave",
    "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml", "eth", "ntilde", "ograve", "oacute", "ocirc", "otilde", "ouml",
    "divide", "oslash", "ugrave", "uacute", "ucirc", "uuml", "yacute", "thorn", "yuml",
];

/// Остальные сущности HTML 4 (и `&numero;`).
const NAMED: &[(&str, u32)] = &[
    ("OElig", 338), ("oelig", 339), ("Scaron", 352), ("scaron", 353), ("Yuml", 376), ("fnof", 402), ("circ", 710), ("tilde", 732),
    ("Alpha", 913), ("Beta", 914), ("Gamma", 915), ("Delta", 916), ("Epsilon", 917), ("Zeta", 918), ("Eta", 919), ("Theta", 920),
    ("Iota", 921), ("Kappa", 922), ("Lambda", 923), ("Mu", 924), ("Nu", 925), ("Xi", 926), ("Omicron", 927), ("Pi", 928),
    ("Rho", 929), ("Sigma", 931), ("Tau", 932), ("Upsilon", 933), ("Phi", 934), ("Chi", 935), ("Psi", 936), ("Omega", 937),
    ("alpha", 945), ("beta", 946), ("gamma", 947), ("delta", 948), ("epsilon", 949), ("zeta", 950), ("eta", 951), ("theta", 952),
    ("iota", 953), ("kappa", 954), ("lambda", 955), ("mu", 956), ("nu", 957), ("xi", 958), ("omicron", 959), ("pi", 960),
    ("rho", 961), ("sigmaf", 962), ("sigma", 963), ("tau", 964), ("upsilon", 965), ("phi", 966), ("chi", 967), ("psi", 968),
    ("omega", 969), ("thetasym", 977), ("upsih", 978), ("piv", 982), ("ensp", 8194), ("emsp", 8195), ("thinsp", 8201),
    ("zwnj", 8204), ("zwj", 8205), ("lrm", 8206), ("rlm", 8207), ("lsquo", 8216), ("rsquo", 8217), ("sbquo", 8218),
    ("ldquo", 8220), ("rdquo", 8221), ("bdquo", 8222), ("dagger", 8224), ("Dagger", 8225), ("bull", 8226), ("permil", 8240),
    ("prime", 8242), ("Prime", 8243), ("lsaquo", 8249), ("rsaquo", 8250), ("oline", 8254), ("frasl", 8260), ("euro", 8364),
    ("image", 8465), ("weierp", 8472), ("real", 8476), ("numero", 8470), ("trade", 8482), ("alefsym", 8501), ("larr", 8592),
    ("uarr", 8593), ("rarr", 8594), ("darr", 8595), ("harr", 8596), ("crarr", 8629), ("lArr", 8656), ("uArr", 8657),
    ("rArr", 8658), ("dArr", 8659), ("hArr", 8660), ("forall", 8704), ("part", 8706), ("exist", 8707), ("empty", 8709),
    ("nabla", 8711), ("isin", 8712), ("notin", 8713), ("ni", 8715), ("prod", 8719), ("sum", 8721), ("minus", 8722),
    ("lowast", 8727), ("radic", 8730), ("prop", 8733), ("infin", 8734), ("ang", 8736), ("and", 8743), ("or", 8744),
    ("cap", 8745), ("cup", 8746), ("int", 8747), ("there4", 8756), ("sim", 8764), ("cong", 8773), ("asymp", 8776), ("ne", 8800),
    ("equiv", 8801), ("le", 8804), ("ge", 8805), ("sub", 8834), ("sup", 8835), ("nsub", 8836), ("sube", 8838), ("supe", 8839),
    ("oplus", 8853), ("otimes", 8855), ("perp", 8869), ("sdot", 8901), ("lceil", 8968), ("rceil", 8969), ("lfloor", 8970),
    ("rfloor", 8971), ("lang", 9001), ("rang", 9002), ("loz", 9674), ("spades", 9824), ("clubs", 9827), ("hearts", 9829),
    ("diams", 9830), ("hyphen", 8208), ("dash", 8208), ("check", 10003), ("star", 9734), ("nbhy", 8209),
];

/// Текст файла XML: по метке порядка байтов, по объявлению `<?xml encoding=…?>`, иначе UTF-8;
/// если это не UTF-8 — windows-1251 (так сохранены многие старые книги FB2).
pub fn decode_xml(data: &[u8]) -> String {
    if let Some((enc, bom)) = encoding_rs::Encoding::for_bom(data) {
        return enc.decode_without_bom_handling(&data[bom..]).0.into_owned();
    }
    if let Some(label) = declared_encoding(data) {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            if enc != encoding_rs::UTF_8 && enc != encoding_rs::UTF_16LE && enc != encoding_rs::UTF_16BE {
                return enc.decode_without_bom_handling(data).0.into_owned();
            }
        }
    }
    match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1251.decode_without_bom_handling(data).0.into_owned(),
    }
}

fn declared_encoding(data: &[u8]) -> Option<String> {
    let head = &data[..data.len().min(300)];
    if !head.starts_with(b"<?xml") {
        return None;
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = String::from_utf8_lossy(&head[..end]);
    let at = decl.find("encoding")?;
    let rest = decl[at + 8..].trim_start().strip_prefix('=')?.trim_start();
    let q = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value: String = rest[1..].chars().take_while(|c| *c != q).collect();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(el: &El) -> Vec<String> {
        el.elements().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn entity_tables() {
        assert_eq!(LATIN1.len(), 95);
        assert_eq!(entity("yuml"), Some('ÿ'));
        assert_eq!(entity("iexcl"), Some('¡'));
        assert_eq!(entity("diams"), Some('♦'));
    }

    #[test]
    fn entities_and_numbers() {
        assert_eq!(decode("a&nbsp;b &mdash; &laquo;в&raquo; &#171;&#xBB; &amp;lt; &unknown; & x"), "a\u{a0}b — «в» «» &lt; &unknown; & x");
        assert_eq!(decode("&#151; &#0; &#xD800; &eacute;&Eacute; &numero;"), "— \u{FFFD} \u{FFFD} éÉ №");
        assert_eq!(decode("без сущностей"), "без сущностей");
    }

    #[test]
    fn xml_names_are_local_and_lowercase() {
        let doc = parse(r#"<?xml version="1.0"?><package><dc:Title id=t1>Пикник</dc:Title><navMap/></package>"#, false);
        let pkg = doc.child("package").unwrap();
        assert_eq!(names(pkg), ["title", "navmap"]);
        assert_eq!(pkg.child("title").unwrap().attr("id"), Some("t1"));
        assert_eq!(pkg.child("title").unwrap().text(), "Пикник");
    }

    #[test]
    fn broken_markup_still_parses() {
        let doc = parse("<body><p>раз<p>два<br>три</div><li>a<li>b</body>", true);
        let body = doc.child("body").unwrap();
        assert_eq!(names(body), ["p", "p", "li", "li"]);
        let second = body.elements().nth(1).unwrap();
        assert_eq!(names(second), ["br"]);
        assert_eq!(second.text(), "дватри");
        // Незакрытое в конце закрывается.
        let doc = parse("<div><span>текст", true);
        assert_eq!(doc.find("span").unwrap().text(), "текст");
    }

    #[test]
    fn comments_cdata_doctype_and_raw_style() {
        let src = "<!DOCTYPE html [<!ENTITY x \"y\">]><html><head><style>p > span { color: red } </p></style></head>\
                   <body><!-- <p>нет</p> --><p><![CDATA[<b>как есть</b>]]></p><script>if (a < b) {}</script></body></html>";
        let doc = parse(src, true);
        assert_eq!(doc.find("style").unwrap().text(), "p > span { color: red } </p>");
        let body = doc.find("body").unwrap();
        assert_eq!(names(body), ["p", "script"]);
        assert_eq!(body.child("p").unwrap().text(), "<b>как есть</b>");
    }

    #[test]
    fn attributes_in_all_forms() {
        let doc = parse(r##"<img src='a b.png' alt=Обложка hidden data-x="1 &amp; 2"/><a xlink:href="#n1" HREF="x">"##, true);
        let img = doc.child("img").unwrap();
        assert_eq!(img.attr("src"), Some("a b.png"));
        assert_eq!(img.attr("alt"), Some("Обложка"));
        assert_eq!(img.attr("hidden"), Some(""));
        assert_eq!(img.attr("data-x"), Some("1 & 2"));
        // Повтор имени атрибута после местного имени — первый побеждает.
        assert_eq!(doc.child("a").unwrap().attr("href"), Some("#n1"));
    }

    #[test]
    fn depth_is_limited() {
        let deep = "<div>".repeat(5000) + "дно" + &"</div>".repeat(5000);
        let doc = parse(&deep, true);
        let mut depth = 0;
        let mut el = &doc;
        while let Some(next) = el.child("div") {
            el = next;
            depth += 1;
        }
        assert!(depth <= MAX_DEPTH);
        assert_eq!(doc.text(), "дно");
    }

    #[test]
    fn encodings() {
        let (cp, _, _) = encoding_rs::WINDOWS_1251.encode("<?xml version=\"1.0\" encoding=\"windows-1251\"?><p>Привет</p>");
        assert!(decode_xml(&cp).contains("Привет"));
        // Без объявления и не UTF-8 — windows-1251.
        let (cp, _, _) = encoding_rs::WINDOWS_1251.encode("<p>Ёлка</p>");
        assert_eq!(decode_xml(&cp), "<p>Ёлка</p>");
        // UTF-16 с меткой.
        let mut u16 = vec![0xFF, 0xFE];
        for unit in "<p>Ок</p>".encode_utf16() {
            u16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_xml(&u16), "<p>Ок</p>");
        assert_eq!(decode_xml("<p>UTF-8 ✓</p>".as_bytes()), "<p>UTF-8 ✓</p>");
    }
}

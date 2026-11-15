//! Номера источников «[N]» в книгах без настоящих сносок.
//!
//! Конвертеры (Fiction Book Designer и другие) часто оставляют «[2]» и «[27, с. 15]» простым текстом,
//! а список литературы — обычной главой с пунктами «1. …». Если своих сносок в книге нет, а в одной
//! из последних глав есть список — глава «Литература», «Источники», «Примечания»… (или раздел с таким
//! заголовком) с пунктами «N. …», «[N] …», нумерованным списком или примечаниями «N», — номера в тексте
//! становятся сносками на эти пункты: нажатие показывает пункт в том же окне, что у сносок FB2.
//! Текст книги не меняется ни на знак — места чтения, закладки и цитаты остаются на своих местах.

use super::tree::{text_of, Out, OutEl};
use std::collections::HashMap;

/// Сколько глав с конца смотреть в поисках списка.
const LOOK_BACK: usize = 8;
/// Пунктов в списке не меньше — иначе это не список литературы.
const MIN_ENTRIES: usize = 3;
/// Самая длинная скобка с номерами: «[12, с. 145–147; 14, т. 2, с. 3]».
const MAX_BRACKET: usize = 60;

/// Начала названий списка (строчными, «ё» как «е»).
const TITLES: [&str; 20] = [
    "литература",
    "список литератур",
    "список использованн",
    "список источник",
    "использованная литература",
    "рекомендуемая литература",
    "цитируемая литература",
    "библиография",
    "источники",
    "примечания",
    "комментарии",
    "сноски",
    "notes",
    "endnotes",
    "references",
    "bibliography",
    "sources",
    "works cited",
    "literature",
    "примечания и литература",
];

fn folded(s: &str) -> String {
    let lower: String = s.chars().flat_map(char::to_lowercase).map(|c| if c == 'ё' { 'е' } else { c }).collect();
    lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn is_list_title(title: &str) -> bool {
    let t = folded(title);
    !t.is_empty() && t.chars().count() <= 60 && TITLES.iter().any(|p| t.starts_with(p))
}

fn text(nodes: &[Out]) -> String {
    let mut s = String::new();
    text_of(nodes, &mut s);
    s
}

/// «12. Поршнев…», «12) …», «[12] …» → 12.
fn leading_number(s: &str) -> Option<u32> {
    let t = s.trim_start();
    let (body, close) = match t.strip_prefix('[') {
        Some(rest) => (rest, ']'),
        None => (t, '.'),
    };
    let digits: String = body.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let mut rest = body[digits.len()..].chars();
    let sep = rest.next()?;
    let ok = if close == ']' { sep == ']' } else { sep == '.' || sep == ')' };
    // После номера — текст пункта: «1.5» или «2.» без текста — не пункт.
    let after: String = rest.collect();
    if !ok || after.trim().is_empty() || (close != ']' && !after.starts_with(char::is_whitespace)) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/// Заголовок примечания — только номер: «1», «1.», «[1]».
fn bare_number(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('[').trim_end_matches([']', '.', ')']);
    (!t.is_empty() && t.len() <= 3 && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok()).flatten().filter(|n| *n > 0)
}

fn heading(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4")
}

/// Обход пунктов списка в одном и том же порядке: первый раз — чтобы их посчитать, второй — чтобы
/// дать им метки. `active` — пункты собираются (глава-список или после заголовка «Литература»).
fn visit(nodes: &mut [Out], active: &mut bool, f: &mut dyn FnMut(u32, &mut OutEl)) {
    for n in nodes {
        let Out::El(e) = n else { continue };
        if heading(e.tag) {
            let title = text(&e.kids);
            if is_list_title(&title) {
                *active = true;
            } else if *active && bare_number(&title).is_none() {
                // Другой заголовок после списка — список кончился.
                *active = false;
            }
            continue;
        }
        if !*active {
            visit(&mut e.kids, active, f);
            continue;
        }
        match e.tag {
            "p" => {
                if let Some(n) = leading_number(&text(&e.kids)) {
                    f(n, e);
                }
            }
            "ol" => {
                let start: u32 = e.get("start").and_then(|s| s.parse().ok()).unwrap_or(1);
                let mut i = 0;
                for k in e.kids.iter_mut() {
                    if let Out::El(li) = k {
                        if li.tag == "li" {
                            f(start + i, li);
                            i += 1;
                        }
                    }
                }
            }
            // Примечание FB2: раздел, у которого первый заголовок или абзац — только номер.
            "section" | "div" => {
                let first = e.kids.iter().find_map(|k| match k {
                    Out::El(x) if heading(x.tag) || x.tag == "p" => Some(text(&x.kids)),
                    _ => None,
                });
                match first.as_deref().and_then(bare_number) {
                    Some(n) => f(n, e),
                    None => visit(&mut e.kids, active, f),
                }
            }
            _ => visit(&mut e.kids, active, f),
        }
    }
}

/// Пункты списка в главе: номер → метка. Метки даются только найденному списку.
fn entries(nodes: &mut [Out], whole: bool, assign: bool) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut active = whole;
    visit(nodes, &mut active, &mut |n, el| {
        if map.contains_key(&n) {
            return;
        }
        let id = match el.get("id") {
            Some(id) => id.to_string(),
            None => {
                let id = format!("polka-ref-{n}");
                if assign {
                    el.set("id", id.clone());
                }
                id
            }
        };
        map.insert(n, id);
    });
    map
}

fn has_notes(nodes: &[Out]) -> bool {
    nodes.iter().any(|n| matches!(n, Out::El(e) if (e.tag == "a" && e.get("note").is_some()) || has_notes(&e.kids)))
}

/// Номера в скобке, которые стоит сделать ссылками: «2» в «[2, с. 15]», «3» и «5» в «[3–5]» и «[3; 5]».
/// Диапазоны байтов — внутри содержимого скобки. `None` — это не скобка с номерами.
fn numbers_in(content: &str) -> Option<Vec<(usize, usize, u32)>> {
    let b = content.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < b.len() && (b[*i] == b' ' || content[*i..].starts_with('\u{a0}')) {
            *i += if b[*i] == b' ' { 1 } else { '\u{a0}'.len_utf8() };
        }
    };
    let number = |i: &mut usize| -> Option<(usize, usize, u32)> {
        let s = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        let len = *i - s;
        (1..=3).contains(&len).then(|| content[s..*i].parse().ok().map(|n| (s, *i, n))).flatten()
    };
    let dash = |i: &mut usize| -> bool {
        for d in ["–", "—", "-"] {
            if content[*i..].starts_with(d) {
                *i += d.len();
                return true;
            }
        }
        false
    };
    // Страницы, том, глава: дальше в этой части скобки номеров источников нет.
    let pages = |i: usize| -> bool {
        let rest: String = content[i..].chars().take(4).flat_map(char::to_lowercase).collect();
        ["с.", "c.", "с ", "стр", "p.", "pp", "s.", "т.", "vol", "гл.", "л.", "ч."].iter().any(|m| rest.starts_with(m))
    };
    loop {
        skip_ws(&mut i);
        out.push(number(&mut i)?);
        loop {
            skip_ws(&mut i);
            if i >= b.len() || b[i] == b';' {
                break;
            }
            if dash(&mut i) {
                skip_ws(&mut i);
                out.push(number(&mut i)?);
                continue;
            }
            if b[i] == b',' {
                i += 1;
                skip_ws(&mut i);
                if pages(i) {
                    // До конца этой части скобки — страницы.
                    while i < b.len() && b[i] != b';' {
                        i += 1;
                    }
                    break;
                }
                out.push(number(&mut i)?);
                continue;
            }
            return None;
        }
        if i >= b.len() {
            return Some(out);
        }
        i += 1; // «;»
    }
}

fn cite_link(label: &str, to: &str) -> Out {
    let mut a = OutEl::new("a").with("to", to).with("note", "1");
    a.kids.push(Out::Text(label.to_string()));
    Out::El(a)
}

/// Текст с номерами в скобках → куски текста и ссылки; `None` — ссылок нет.
fn split(t: &str, map: &HashMap<u32, String>, ch: usize, made: &mut usize) -> Option<Vec<Out>> {
    let mut pieces: Vec<Out> = Vec::new();
    let mut last = 0;
    let mut from = 0;
    while let Some(open) = t[from..].find('[').map(|p| p + from) {
        let close = match t[open + 1..].find([']', '[']) {
            Some(p) if t.as_bytes()[open + 1 + p] == b']' && p <= MAX_BRACKET => open + 1 + p,
            _ => {
                from = open + 1;
                continue;
            }
        };
        let content = &t[open + 1..close];
        let found: Vec<(usize, usize, u32)> = numbers_in(content).unwrap_or_default().into_iter().filter(|(_, _, n)| map.contains_key(n)).collect();
        if found.is_empty() {
            from = close + 1;
            continue;
        }
        if open > last {
            pieces.push(Out::Text(t[last..open].to_string()));
        }
        let to = |n: u32| format!("{ch}#{}", map[&n]);
        if found.len() == 1 && numbers_in(content).is_some_and(|all| all.len() == 1) {
            // Один источник — ссылка вся скобка: в неё проще попасть пальцем.
            pieces.push(cite_link(&t[open..=close], &to(found[0].2)));
        } else {
            let mut pos = open + 1;
            pieces.push(Out::Text("[".into()));
            for (s, e, n) in &found {
                let (s, e) = (open + 1 + s, open + 1 + e);
                if s > pos {
                    pieces.push(Out::Text(t[pos..s].to_string()));
                }
                pieces.push(cite_link(&t[s..e], &to(*n)));
                pos = e;
            }
            pieces.push(Out::Text(t[pos..=close].to_string()));
        }
        *made += found.len();
        last = close + 1;
        from = close + 1;
    }
    if pieces.is_empty() {
        return None;
    }
    if last < t.len() {
        pieces.push(Out::Text(t[last..].to_string()));
    }
    Some(pieces)
}

fn link_nodes(nodes: &mut Vec<Out>, map: &HashMap<u32, String>, ch: usize, made: &mut usize) {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i] {
            Out::El(e) => {
                if !matches!(e.tag, "a" | "code" | "h1" | "h2" | "h3" | "h4") {
                    link_nodes(&mut e.kids, map, ch, made);
                }
                i += 1;
            }
            Out::Text(t) => match split(t, map, ch, made) {
                Some(pieces) => {
                    let n = pieces.len();
                    nodes.splice(i..=i, pieces);
                    i += n;
                }
                None => i += 1,
            },
        }
    }
}

/// Номера «[N]» → сноски на пункты списка. Ссылки — главой и меткой (`3#polka-ref-12`), как у сносок FB2.
/// Возвращает, сколько номеров стали ссылками.
pub(super) fn link(parts: &mut [(String, Vec<Out>)]) -> usize {
    if parts.iter().any(|(_, n)| has_notes(n)) {
        return 0;
    }
    let from = parts.len().saturating_sub(LOOK_BACK);
    let found = (from..parts.len()).rev().find_map(|i| {
        let whole = is_list_title(&parts[i].0);
        let map = entries(&mut parts[i].1, whole, false);
        let first = map.keys().min().copied().unwrap_or(u32::MAX);
        (map.len() >= MIN_ENTRIES && first <= 2).then_some((i, whole))
    });
    let Some((list, whole)) = found else { return 0 };
    let map = entries(&mut parts[list].1, whole, true);
    let mut made = 0;
    for (i, (_, nodes)) in parts.iter_mut().enumerate() {
        if i != list {
            link_nodes(nodes, &map, list, &mut made);
        }
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::tree::to_json;

    fn p(s: &str) -> Out {
        let mut e = OutEl::new("p");
        e.kids.push(Out::Text(s.into()));
        Out::El(e)
    }

    fn book(text: &str) -> Vec<(String, Vec<Out>)> {
        vec![
            ("Глава первая".into(), vec![p(text)]),
            ("ЛИТЕРАТУРА".into(), vec![p("1. Б.Ф. Поршнев, «О начале человеческой истории». М., 1974."), p("2. Пьер Тейяр де Шарден, «Феномен человека». М., 1965."), p("3. Торстейн Веблен, «Теория праздного класса». М., 1983."), p("4. Бруно Оля, «Боги Тропической Африки». М., 1977.")]),
        ]
    }

    fn all_text(parts: &[(String, Vec<Out>)]) -> String {
        parts.iter().map(|(_, n)| text(n)).collect()
    }

    #[test]
    fn numbers_in_brackets_point_to_the_list() {
        let src = "Как писал Поршнев [1], «специализация убивает» [2, с. 15], см. [3; 4] и [2–4]. Год [1917], [sic], [5] и [1.5].";
        let mut parts = book(src);
        let before = all_text(&parts);
        assert_eq!(link(&mut parts), 1 + 1 + 2 + 2);
        assert_eq!(all_text(&parts), before, "текст книги не меняется ни на знак");
        let json = to_json(&parts[0].1);
        assert!(json.contains(r#"["a",{"to":"1#polka-ref-1","note":"1"},"[1]"]"#), "{json}");
        assert!(json.contains(r#"["a",{"to":"1#polka-ref-2","note":"1"},"[2, с. 15]"]"#), "{json}");
        assert!(json.contains(r#""[",["a",{"to":"1#polka-ref-3","note":"1"},"3"],"; ",["a",{"to":"1#polka-ref-4","note":"1"},"4"],"]""#), "{json}");
        assert!(json.contains(r#"["a",{"to":"1#polka-ref-2","note":"1"},"2"],"–",["a",{"to":"1#polka-ref-4","note":"1"},"4"]"#), "{json}");
        assert!(json.contains("Год [1917], [sic], [5] и [1.5]."), "чужие скобки и номера без пункта — как были: {json}");
        assert!(to_json(&parts[1].1).contains(r#"{"id":"polka-ref-2"}"#), "у пунктов списка — метки");
    }

    #[test]
    fn real_notes_or_no_list_leave_the_book_alone() {
        let mut parts = book("Текст [1].");
        let mut a = OutEl::new("a").with("to", "1#n1").with("note", "1");
        a.kids.push(Out::Text("1".into()));
        parts[0].1.push(Out::El(a));
        let before = parts.clone();
        assert_eq!(link(&mut parts), 0, "свои сноски в книге есть — догадки не нужны");
        assert_eq!(parts, before);
        let mut parts = vec![("Глава".into(), vec![p("Текст [1] и [2].")]), ("Послесловие".into(), vec![p("1. Раз."), p("2. Два."), p("3. Три.")])];
        assert_eq!(link(&mut parts), 0, "глава не похожа на список литературы");
    }

    #[test]
    fn list_under_a_heading_numbered_list_and_note_sections() {
        // Список — разделом в конце последней главы.
        let mut h = OutEl::new("h2");
        h.kids.push(Out::Text("Список литературы".into()));
        let mut parts = vec![("Глава".into(), vec![p("Начало [2] и [3]."), Out::El(h), p("1. Раз."), p("2. Два."), p("3. Три.")])];
        assert_eq!(link(&mut parts), 0, "ссылки внутри той же главы, что и список, не ставим");
        let mut h = OutEl::new("h2");
        h.kids.push(Out::Text("Список литературы".into()));
        let mut parts = vec![("Глава".into(), vec![p("Начало [2] и [3].")]), ("Заключение".into(), vec![p("Итог."), Out::El(h), p("1. Раз."), p("2. Два."), p("3. Три.")])];
        assert_eq!(link(&mut parts), 2);
        // Нумерованный список EPUB.
        let mut ol = OutEl::new("ol");
        for s in ["Раз.", "Два.", "Три."] {
            let mut li = OutEl::new("li");
            li.kids.push(Out::Text(s.into()));
            ol.kids.push(Out::El(li));
        }
        let mut parts = vec![("Глава".into(), vec![p("См. [3].")]), ("References".into(), vec![Out::El(ol)])];
        assert_eq!(link(&mut parts), 1);
        assert!(to_json(&parts[0].1).contains(r#""to":"1#polka-ref-3""#));
        // Примечания разделами с номером в заголовке (тело примечаний FB2 без ссылок на него).
        let note = |n: &str, id: &str| {
            let mut s = OutEl::new("section").with("id", id);
            let mut t = OutEl::new("h3");
            t.kids.push(Out::Text(n.into()));
            s.kids.push(Out::El(t));
            s.kids.push(p("Пояснение."));
            Out::El(s)
        };
        let mut parts = vec![("Глава".into(), vec![p("Текст [1], [2] и [3].")]), ("Примечания".into(), vec![note("1", "n_1"), note("2", "n_2"), note("3", "n_3")])];
        assert_eq!(link(&mut parts), 3);
        assert!(to_json(&parts[0].1).contains(r#""to":"1#n_2""#), "своя метка примечания сохраняется");
    }

    #[test]
    fn bracket_grammar() {
        let nums = |s: &str| numbers_in(s).map(|v| v.into_iter().map(|(_, _, n)| n).collect::<Vec<_>>());
        assert_eq!(nums("2"), Some(vec![2]));
        assert_eq!(nums("2, с. 15"), Some(vec![2]));
        assert_eq!(nums("12, с. 145–147; 14, т. 2, с. 3"), Some(vec![12, 14]));
        assert_eq!(nums("3–5"), Some(vec![3, 5]));
        assert_eq!(nums("2, 5, 7"), Some(vec![2, 5, 7]));
        assert_eq!(nums("1917"), None);
        assert_eq!(nums("sic"), None);
        assert_eq!(nums("1.5"), None);
        assert_eq!(nums("2, см. выше"), None);
        assert_eq!(leading_number("12. Поршнев"), Some(12));
        assert_eq!(leading_number("[3] Веблен"), Some(3));
        assert_eq!(leading_number("1.5 метра"), None);
        assert_eq!(leading_number("1917. Революция"), None);
        assert!(is_list_title("ЛИТЕРАТУРА") && is_list_title("Список использованной литературы") && is_list_title("Примечания"));
        assert!(!is_list_title("Русская литература XIX века") && !is_list_title("Послесловие"));
    }
}

//! Книга из файла и полка: найти ту же книгу среди записей или завести новую.

use super::Meta;
use crate::model::{Format, Kind, Status, Title};
use crate::Date;

/// Сколько знаков считать страницей — для «Итогов» у электронных книг без числа страниц.
pub const CHARS_PER_PAGE: usize = 1800;

/// Для сравнения: нижний регистр, «ё» → «е», только буквы и цифры через пробел.
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.chars().flat_map(char::to_lowercase) {
        let c = if c == 'ё' { 'е' } else { c };
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim_end().to_string()
}

fn same_title(shelf: &str, file: &str) -> bool {
    let (a, b) = (norm(shelf), norm(file));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    // «Пикник на обочине» и «Пикник на обочине. Повесть».
    a == b || (a.chars().count() >= 8 && b.starts_with(&format!("{a} "))) || (b.chars().count() >= 8 && a.starts_with(&format!("{b} ")))
}

/// Общая часть имени длиной от трёх букв: «Стругацкий А.» и «Аркадий Стругацкий».
fn same_people(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    let words = |list: &[String]| -> Vec<String> { list.iter().flat_map(|n| norm(n).split(' ').map(str::to_string).collect::<Vec<_>>()).filter(|w| w.chars().count() >= 3).collect() };
    let wa = words(a);
    let wb = words(b);
    wa.iter().any(|w| wb.contains(w))
}

/// Запись полки для книги из файла: тот же ISBN или то же название и автор.
/// Из нескольких — та, что читается сейчас, потом «Хочу», потом остальные.
pub fn find(titles: &[Title], meta: &Meta) -> Option<String> {
    let books = titles.iter().filter(|t| t.kind == Kind::Book);
    let mut found: Vec<&Title> = match &meta.isbn {
        Some(isbn) => books.clone().filter(|t| t.isbn.as_deref() == Some(isbn)).collect(),
        None => Vec::new(),
    };
    if found.is_empty() && !meta.title.is_empty() {
        found = books
            .filter(|t| (same_title(&t.title, &meta.title) || t.original.as_deref().is_some_and(|o| same_title(o, &meta.title))) && same_people(&t.creators, &meta.authors))
            .collect();
    }
    let rank = |t: &&Title| match t.status {
        Status::Doing => 0,
        Status::Want => 1,
        Status::Done => 2,
        Status::Dropped => 3,
    };
    found.into_iter().min_by_key(rank).map(|t| t.id.clone())
}

/// Новая запись «Читаю» по описанию из файла (без номера и обложки — их ставит вызывающий).
pub fn new_title(meta: &Meta, chars: usize, file_name: &str, today: Date) -> Title {
    let name = if meta.title.trim().is_empty() { file_stem(file_name) } else { meta.title.clone() };
    let mut t = Title::new(Kind::Book, &super::clip(&name, 300), today);
    t.creators = meta.authors.clone();
    t.year = meta.year.clone();
    t.publisher = meta.publisher.clone();
    t.isbn = meta.isbn.clone();
    t.about = meta.about.clone();
    t.genres = meta.genres.clone();
    t.format = Some(Format::Ebook);
    if chars > 0 {
        t.pages = Some(chars.div_ceil(CHARS_PER_PAGE).max(1) as u32);
    }
    t.set_status(Status::Doing, today);
    t
}

/// Начать чтение записи: «Хочу» → «В процессе», формат — электронная, число страниц — по объёму.
/// `true`, если запись изменилась.
pub fn start_reading(t: &mut Title, chars: usize, today: Date) -> bool {
    let before = t.clone();
    if t.status == Status::Want {
        t.set_status(Status::Doing, today);
    }
    if t.format.is_none() {
        t.format = Some(Format::Ebook);
    }
    if t.pages.is_none() && chars > 0 && t.format == Some(Format::Ebook) {
        t.pages = Some(chars.div_ceil(CHARS_PER_PAGE).max(1) as u32);
    }
    *t != before
}

fn file_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    let cut = [".fb2.zip", ".epub", ".fb2", ".zip"].iter().find(|e| lower.ends_with(*e)).map(|e| base.len() - e.len()).unwrap_or(base.len());
    let stem = base[..cut].replace(['_', '.'], " ");
    let stem = super::squash(&stem);
    if stem.is_empty() {
        "Без названия".into()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: &str, title: &str, authors: &[&str], status: Status) -> Title {
        let mut t = Title::new(Kind::Book, title, Date::parse("2026-01-01").unwrap());
        t.id = id.into();
        t.creators = authors.iter().map(|s| s.to_string()).collect();
        t.status = status;
        t
    }

    #[test]
    fn finds_the_same_book() {
        let shelf = vec![
            book("a", "Пикник на обочине", &["Стругацкий А."], Status::Done),
            book("b", "Пикник на обочине", &["Аркадий и Борис Стругацкие"], Status::Want),
            book("c", "Трудно быть богом", &["Аркадий Стругацкий"], Status::Doing),
        ];
        let meta = Meta { title: "Пикник на обочине. Повесть".into(), authors: vec!["Аркадий Стругацкий".into(), "Борис Стругацкий".into()], ..Meta::default() };
        // Подходят обе записи «Пикника» (общие «стругацкий» и «аркадий»): «Хочу» важнее прочитанной.
        assert_eq!(find(&shelf, &meta).as_deref(), Some("b"));
        assert_eq!(find(&shelf[..1], &meta).as_deref(), Some("a"));
        let meta = Meta { title: "ТРУДНО БЫТЬ БОГОМ".into(), authors: vec![], ..Meta::default() };
        assert_eq!(find(&shelf, &meta).as_deref(), Some("c"));
        let meta = Meta { title: "Трудно быть богом".into(), authors: vec!["Лем".into()], ..Meta::default() };
        assert_eq!(find(&shelf, &meta), None);
        let mut with_isbn = book("d", "Другое название", &[], Status::Want);
        with_isbn.isbn = Some("9785170906307".into());
        let meta = Meta { title: "Совсем иное".into(), isbn: Some("9785170906307".into()), ..Meta::default() };
        assert_eq!(find(&[with_isbn], &meta).as_deref(), Some("d"));
    }

    #[test]
    fn new_record_from_file() {
        let today = Date::parse("2026-11-14").unwrap();
        let meta = Meta { title: "Солярис".into(), authors: vec!["Станислав Лем".into()], year: Some("1961".into()), ..Meta::default() };
        let t = new_title(&meta, 360_000, "solaris.epub", today);
        assert_eq!((t.title.as_str(), t.status, t.started, t.pages, t.format), ("Солярис", Status::Doing, Some(today), Some(200), Some(Format::Ebook)));
        assert!(t.check().is_ok());
        let t = new_title(&Meta::default(), 0, "C:\\Книги\\Strugatsky_Piknik.fb2.zip", today);
        assert_eq!(t.title, "Strugatsky Piknik");
        assert_eq!(t.pages, None);
    }

    #[test]
    fn starting_to_read_moves_want_to_doing() {
        let today = Date::parse("2026-11-14").unwrap();
        let mut t = book("a", "Солярис", &[], Status::Want);
        assert!(start_reading(&mut t, 18_000, today));
        assert_eq!((t.status, t.started, t.pages), (Status::Doing, Some(today), Some(10)));
        assert!(!start_reading(&mut t, 18_000, today));
        // Бумажная книга остаётся бумажной, страницы не трогаем.
        let mut paper = book("b", "Солярис", &[], Status::Done);
        paper.format = Some(Format::Paper);
        assert!(!start_reading(&mut paper, 18_000, today));
    }
}

//! ISBN: проверка контрольной цифры, перевод ISBN-10 в ISBN-13, штрихкод книги.

/// Цифры и X — всё остальное (дефисы, пробелы, «ISBN») выбрасывается.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x').map(|c| c.to_ascii_uppercase()).collect()
}

fn valid10(d: &str) -> bool {
    if d.len() != 10 || !d[..9].bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let mut sum = 0u32;
    for (i, b) in d.bytes().enumerate() {
        let v = match b {
            b'X' if i == 9 => 10,
            b'0'..=b'9' => (b - b'0') as u32,
            _ => return false,
        };
        sum += v * (10 - i as u32);
    }
    sum.is_multiple_of(11)
}

fn check13(first12: &str) -> u32 {
    let sum: u32 = first12.bytes().enumerate().map(|(i, b)| (b - b'0') as u32 * if i % 2 == 0 { 1 } else { 3 }).sum();
    (10 - sum % 10) % 10
}

fn valid13(d: &str) -> bool {
    d.len() == 13 && d.bytes().all(|b| b.is_ascii_digit()) && check13(&d[..12]) == (d.as_bytes()[12] - b'0') as u32
}

/// ISBN-13 из любой записи ISBN-10 или ISBN-13; `None`, если номер неверный.
pub fn normalize(s: &str) -> Option<String> {
    let d = clean(s);
    if valid13(&d) && (d.starts_with("978") || d.starts_with("979")) {
        return Some(d);
    }
    if valid10(&d) {
        let body = format!("978{}", &d[..9]);
        let c = check13(&body);
        return Some(format!("{body}{c}"));
    }
    None
}

/// Штрихкод с обложки — книга, если это EAN-13 с префиксом 978 или 979.
pub fn is_book_barcode(s: &str) -> bool {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    d.len() == 13 && valid13(&d) && (d.starts_with("978") || d.starts_with("979"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn13_and_10() {
        assert_eq!(normalize("978-0-306-40615-7").as_deref(), Some("9780306406157"));
        assert_eq!(normalize("0-306-40615-2").as_deref(), Some("9780306406157"));
        assert_eq!(normalize("ISBN 0306406152").as_deref(), Some("9780306406157"));
        assert_eq!(normalize("978-0-306-40615-8"), None);
        assert_eq!(normalize("пикник на обочине"), None);
        assert!(is_book_barcode("9780306406157"));
        assert!(!is_book_barcode("4600000000003"));
    }

    #[test]
    fn isbn10_with_x() {
        // 0-8044-2957-X — классический пример с X в конце.
        assert_eq!(normalize("0-8044-2957-X").as_deref(), Some("9780804429573"));
    }
}

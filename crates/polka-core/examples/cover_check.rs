//! Проверка обложек (этап 0): у скольких книг из списка каталоги находят обложку,
//! и скачивается ли она на самом деле.
//!
//! ```text
//! cargo run -p polka-core --example cover_check -- docs/cover-check-books.txt
//! ```
//!
//! Фильмы и сериалы — с ключом ПоискКино (из Telegram-бота @poiskkinodev_bot):
//!
//! ```text
//! POISKKINO_KEY=… cargo run -p polka-core --example cover_check -- docs/cover-check-books.txt docs/cover-check-films.txt
//! ```
//!
//! Ключ Google Books нужен: без него Google не отвечает — `GOOGLE_BOOKS_KEY=…`.
//! Итог печатается в консоль; подробности с картинками — `cover-report.html`, данные — `cover-report.json`.

use polka_core::catalog::{self, http::Net, Candidate, Keys};
use serde_json::json;
use std::time::Duration;

struct Row {
    line: String,
    ru: bool,
    found: Option<Candidate>,
    /// Какая обложка скачалась: номер варианта.
    downloaded: Option<usize>,
    failed: Vec<String>,
}

fn read_list(path: &str) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("не открыть {path}: {e}"));
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()
}

fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

/// «Название (1997)» → «Название»; «Название — Автор» → «Название Автор».
fn query_of(line: &str) -> String {
    let no_year = match line.find(" (") {
        Some(i) if line.ends_with(')') => &line[..i],
        _ => line,
    };
    no_year.replace(" — ", " ").replace(" - ", " ")
}

/// Скачать первую рабочую обложку из вариантов (не больше трёх попыток).
fn try_download(net: &Net, c: &Candidate) -> Option<usize> {
    for (i, cov) in c.covers.iter().enumerate().take(3) {
        if catalog::fetch_cover(net, &cov.url).is_ok() {
            return Some(i);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    None
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let books_path = args.next().unwrap_or_else(|| "docs/cover-check-books.txt".into());
    let films_path = args.next();
    let keys = Keys { google: std::env::var("GOOGLE_BOOKS_KEY").ok(), poiskkino: std::env::var("POISKKINO_KEY").ok() };
    let net = Net::new();

    let mut rows: Vec<Row> = Vec::new();
    let books = read_list(&books_path);
    for (i, line) in books.iter().enumerate() {
        let found = catalog::search_books(&net, &query_of(line), &keys);
        let failed: Vec<String> = found.failed.iter().map(|f| format!("{}: {}", f.source, f.message)).collect();
        let best = found.results.into_iter().next().map(|c| catalog::details(&net, &c, &keys));
        let downloaded = best.as_ref().and_then(|c| try_download(&net, c));
        let mark = if downloaded.is_some() { "есть" } else if best.as_ref().is_some_and(|c| !c.covers.is_empty()) { "не скачалась" } else { "нет" };
        eprintln!("[{}/{}] {line} — обложка: {mark}", i + 1, books.len());
        rows.push(Row { ru: has_cyrillic(line), line: line.clone(), found: best, downloaded, failed });
        // FantLab просит не чаще одного запроса в 1,2 секунды.
        std::thread::sleep(Duration::from_millis(1300));
    }

    let mut film_rows: Vec<Row> = Vec::new();
    if let Some(path) = films_path {
        if keys.poiskkino.as_deref().is_some_and(|k| !k.is_empty()) {
            let films = read_list(&path);
            for (i, line) in films.iter().enumerate() {
                let found = catalog::search_films(&net, &query_of(line), &keys);
                let failed: Vec<String> = found.failed.iter().map(|f| format!("{}: {}", f.source, f.message)).collect();
                let best = found.results.into_iter().next();
                let downloaded = best.as_ref().and_then(|c| try_download(&net, c));
                eprintln!("[{}/{}] {line} — постер: {}", i + 1, films.len(), if downloaded.is_some() { "есть" } else { "нет" });
                film_rows.push(Row { ru: has_cyrillic(line), line: line.clone(), found: best, downloaded, failed });
            }
        } else {
            eprintln!("Фильмы пропущены: нет ключа ПоискКино (POISKKINO_KEY).");
        }
    }

    // Итоги.
    let ok = |r: &&&Row| r.downloaded.is_some();
    let ru: Vec<&Row> = rows.iter().filter(|r| r.ru).collect();
    let en: Vec<&Row> = rows.iter().filter(|r| !r.ru).collect();
    let ru_ok = ru.iter().filter(ok).count();
    let en_ok = en.iter().filter(ok).count();
    let total_ok = ru_ok + en_ok;
    let pct = |a: usize, b: usize| (a * 100).checked_div(b).unwrap_or(0);
    let sources = ["FantLab", "Google Books", "Open Library"];
    let by_source: Vec<(String, usize)> = sources
        .iter()
        .map(|s| (s.to_string(), rows.iter().filter(|r| r.found.as_ref().is_some_and(|c| c.covers.iter().any(|x| x.source == *s))).count()))
        .collect();
    println!();
    println!("Обложка скачалась: {total_ok} из {} книг ({}%)", rows.len(), pct(total_ok, rows.len()));
    println!("  русские издания: {ru_ok} из {}", ru.len());
    println!("  на английском:   {en_ok} из {}", en.len());
    for (s, n) in &by_source {
        println!("  {s}: варианты обложки у {n} книг");
    }
    if !film_rows.is_empty() {
        let f_ok = film_rows.iter().filter(|r| r.downloaded.is_some()).count();
        println!("Постер скачался: {f_ok} из {} фильмов и сериалов", film_rows.len());
    }
    let pass = !rows.is_empty() && total_ok * 10 >= rows.len() * 8;
    println!("Цель этапа 0 (обложка минимум у 8 книг из 10): {}", if pass { "выполнена" } else { "не выполнена" });

    // Отчёт с картинками.
    let mut html = String::from(
        "<!doctype html><html lang=\"ru\"><meta charset=\"utf-8\"><title>Проверка обложек «Полки»</title>\
         <style>body{font:15px/1.4 system-ui,sans-serif;margin:24px;color:#15232d}table{border-collapse:collapse;width:100%}\
         td,th{border-top:1px solid #cdd6da;padding:8px;vertical-align:top;text-align:left}img{height:120px;margin:0 6px 6px 0;border-radius:4px}\
         .ok{color:#0b6b57}.no{color:#a4262c}.muted{color:#566671;font-size:13px}figure{display:inline-block;margin:0}figcaption{font-size:12px;color:#566671;max-width:110px}</style>",
    );
    html.push_str(&format!(
        "<h1>Проверка обложек</h1><p><b>{total_ok} из {}</b> книг: русские {ru_ok} из {}, английские {en_ok} из {}. {}</p>",
        rows.len(),
        ru.len(),
        en.len(),
        if pass { "Цель этапа 0 выполнена." } else { "Цель этапа 0 (8 из 10) не выполнена." }
    ));
    for (title, list) in [("Книги", &rows), ("Фильмы и сериалы", &film_rows)] {
        if list.is_empty() {
            continue;
        }
        html.push_str(&format!("<h2>{title}</h2><table><tr><th>Запрос</th><th>Что нашлось</th><th>Обложки</th></tr>"));
        for r in list.iter() {
            let (found, covers) = match &r.found {
                Some(c) => (
                    format!(
                        "{}<br><span class=\"muted\">{} · {} · {}</span>",
                        esc(&c.title),
                        esc(&c.creators.join(", ")),
                        esc(c.year.as_deref().unwrap_or("")),
                        esc(&c.sources.join(", "))
                    ),
                    c.covers
                        .iter()
                        .map(|x| format!("<figure><img src=\"{}\" loading=\"lazy\" alt=\"\"><figcaption>{} {}</figcaption></figure>", esc(&x.url), esc(&x.source), esc(&x.edition)))
                        .collect::<String>(),
                ),
                None => ("<span class=\"no\">не нашлось</span>".to_string(), String::new()),
            };
            let status = if r.downloaded.is_some() { "<span class=\"ok\">обложка скачалась</span>" } else { "<span class=\"no\">обложка не скачалась</span>" };
            let failed = if r.failed.is_empty() { String::new() } else { format!("<br><span class=\"muted\">{}</span>", esc(&r.failed.join("; "))) };
            html.push_str(&format!("<tr><td>{}<br>{status}{failed}</td><td>{found}</td><td>{covers}</td></tr>", esc(&r.line)));
        }
        html.push_str("</table>");
    }
    html.push_str("</html>");
    std::fs::write("cover-report.html", html).expect("не записать cover-report.html");

    let data = json!({
        "books": rows.len(), "books_ok": total_ok, "ru": ru.len(), "ru_ok": ru_ok, "en": en.len(), "en_ok": en_ok,
        "by_source": by_source.iter().map(|(s, n)| json!({"source": s, "books_with_covers": n})).collect::<Vec<_>>(),
        "films": film_rows.len(), "films_ok": film_rows.iter().filter(|r| r.downloaded.is_some()).count(),
        "pass": pass,
        "rows": rows.iter().chain(film_rows.iter()).map(|r| json!({
            "query": r.line, "found": r.found.as_ref().map(|c| &c.title), "sources": r.found.as_ref().map(|c| &c.sources),
            "covers": r.found.as_ref().map(|c| c.covers.len()), "downloaded": r.downloaded, "failed": r.failed,
        })).collect::<Vec<_>>(),
    });
    std::fs::write("cover-report.json", serde_json::to_string_pretty(&data).unwrap_or_default()).expect("не записать cover-report.json");
    println!("Подробности: cover-report.html");
}

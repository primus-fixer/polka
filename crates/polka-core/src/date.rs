//! Календарная дата без часового пояса: для записей важны только дни.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

const GEN: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря",
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Date {
    days: i32, // дней от 1970-01-01
}

impl Date {
    pub fn new(y: i32, m: u32, d: u32) -> Option<Date> {
        if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(Date { days: days_from_civil(y, m, d) })
    }

    /// «2026-10-31»
    pub fn parse(s: &str) -> Option<Date> {
        let mut it = s.trim().splitn(3, '-');
        let y = it.next()?.parse().ok()?;
        let m = it.next()?.parse().ok()?;
        let d = it.next()?.parse().ok()?;
        Date::new(y, m, d)
    }

    /// Дата по числу дней от 1970-01-01 (для «сегодня» из системных часов).
    pub fn from_epoch_days(days: i32) -> Date {
        Date { days }
    }

    /// Сегодняшняя дата по UTC.
    pub fn today_utc() -> Date {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Date { days: (secs / 86_400) as i32 }
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.days)
    }

    /// Сколько дней от `self` до `other` (может быть отрицательным).
    pub fn days_until(self, other: Date) -> i32 {
        other.days - self.days
    }

    pub fn add_days(self, n: i32) -> Date {
        Date { days: self.days + n }
    }

    /// Тот же день через `n` месяцев (последний день месяца, если такого числа нет).
    pub fn add_months(self, n: i32) -> Date {
        let (y, m, d) = self.ymd();
        let total = y * 12 + (m as i32 - 1) + n;
        let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
        let nd = d.min(days_in_month(ny, nm));
        Date { days: days_from_civil(ny, nm, nd) }
    }

    /// День недели: 1 — понедельник, 7 — воскресенье.
    pub fn weekday(self) -> u32 {
        // 1 января 1970 года — четверг.
        (self.days + 3).rem_euclid(7) as u32 + 1
    }

    /// Первое число месяца.
    pub fn month_start(self) -> Date {
        let (y, m, _) = self.ymd();
        Date { days: days_from_civil(y, m, 1) }
    }

    /// Последнее число месяца.
    pub fn month_end(self) -> Date {
        let (y, m, _) = self.ymd();
        Date { days: days_from_civil(y, m, days_in_month(y, m)) }
    }

    /// «31.10.2026» — как в 1С.
    pub fn dotted(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{d:02}.{m:02}.{y}")
    }

    /// «31 октября»
    pub fn day_month(self) -> String {
        let (_, m, d) = self.ymd();
        format!("{d}\u{a0}{}", GEN[(m - 1) as usize])
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Date::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("дата «{s}» не в формате ГГГГ-ММ-ДД")))
    }
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

// Алгоритмы Говарда Хиннанта: дни от эпохи и обратно.
fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_differences() {
        let a = Date::parse("2026-10-21").unwrap();
        let b = Date::parse("2026-10-31").unwrap();
        assert_eq!(a.days_until(b), 10);
        assert_eq!(b.to_string(), "2026-10-31");
        assert_eq!(b.dotted(), "31.10.2026");
        assert_eq!(b.day_month(), "31\u{a0}октября");
        assert_eq!(Date::parse("2024-02-29").unwrap().add_days(1).to_string(), "2024-03-01");
        assert_eq!(Date::parse("2026-01-31").unwrap().add_months(1).to_string(), "2026-02-28");
        assert_eq!(Date::parse("2026-11-15").unwrap().add_months(1).to_string(), "2026-12-15");
        assert!(Date::parse("2026-02-30").is_none());
        assert!(Date::parse("31.10.2026").is_none());
    }
}

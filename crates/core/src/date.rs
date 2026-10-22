//! Мінімальна календарна дата без часових поясів (для віку драйвера/BIOS).

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Дата за григоріанським календарем (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CivilDate {
    /// Рік, напр. 2026.
    pub year: i32,
    /// Місяць 1–12.
    pub month: u8,
    /// День 1–31.
    pub day: u8,
}

/// Різниця між епохою FILETIME (1601-01-01) і Unix (1970-01-01) у секундах.
const FILETIME_UNIX_DIFF_SECS: i64 = 11_644_473_600;

impl CivilDate {
    /// Створює дату з перевіркою діапазонів місяця й дня.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Сьогоднішня дата (UTC) за системним годинником.
    pub fn today() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::from_unix_days(i64::try_from(secs / 86_400).unwrap_or(0))
    }

    /// Дата з кількості днів від 1970-01-01 (алгоритм Howard Hinnant `civil_from_days`).
    pub fn from_unix_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: i32::try_from(year).unwrap_or(i32::MAX),
            // Значення гарантовано в діапазонах 1..=12 і 1..=31 за побудовою алгоритму.
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
        }
    }

    /// Кількість днів від 1970-01-01 (алгоритм `days_from_civil`).
    pub fn to_unix_days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Дата з Windows FILETIME (інтервали по 100 нс від 1601-01-01).
    pub fn from_filetime(filetime: u64) -> Self {
        let secs =
            i64::try_from(filetime / 10_000_000).unwrap_or(i64::MAX) - FILETIME_UNIX_DIFF_SECS;
        Self::from_unix_days(secs.div_euclid(86_400))
    }

    /// Розбирає дату у форматі `MM/DD/YYYY` (так її зберігають SMBIOS і драйвери).
    pub fn parse_mdy(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('/');
        let month = parts.next()?.trim().parse().ok()?;
        let day = parts.next()?.trim().parse().ok()?;
        let year_raw: i32 = parts.next()?.trim().parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        // Старі BIOS іноді пишуть дворічний рік.
        let year = if year_raw < 100 {
            if year_raw < 80 {
                2000 + year_raw
            } else {
                1900 + year_raw
            }
        } else {
            year_raw
        };
        Self::new(year, month, day)
    }

    /// Кількість днів від `self` до `later` (від'ємна, якщо `later` раніше).
    pub fn days_until(self, later: Self) -> i64 {
        later.to_unix_days() - self.to_unix_days()
    }
}

impl std::fmt::Display for CivilDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}.{:02}.{:04}", self.day, self.month, self.year)
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_days_roundtrip() {
        for days in [-1000, 0, 1, 59, 365, 10_957, 20_000, 30_000] {
            assert_eq!(CivilDate::from_unix_days(days).to_unix_days(), days);
        }
        assert_eq!(
            CivilDate::from_unix_days(0),
            CivilDate::new(1970, 1, 1).unwrap()
        );
        assert_eq!(
            CivilDate::from_unix_days(11_016),
            CivilDate::new(2000, 2, 29).unwrap()
        );
    }

    #[test]
    fn filetime_conversion() {
        // 2024-01-01T00:00:00Z
        let ft = (1_704_067_200u64 + 11_644_473_600) * 10_000_000;
        assert_eq!(
            CivilDate::from_filetime(ft),
            CivilDate::new(2024, 1, 1).unwrap()
        );
    }

    #[test]
    fn parse_mdy_formats() {
        assert_eq!(
            CivilDate::parse_mdy("06/21/2006"),
            CivilDate::new(2006, 6, 21)
        );
        assert_eq!(CivilDate::parse_mdy(" 1/2/24 "), CivilDate::new(2024, 1, 2));
        assert_eq!(CivilDate::parse_mdy("13/01/2020"), None);
        assert_eq!(CivilDate::parse_mdy("02/30/2020"), None);
        assert_eq!(CivilDate::parse_mdy("garbage"), None);
    }

    #[test]
    fn days_between() {
        let a = CivilDate::new(2025, 12, 31).unwrap();
        let b = CivilDate::new(2026, 1, 31).unwrap();
        assert_eq!(a.days_until(b), 31);
        assert_eq!(b.days_until(a), -31);
    }
}

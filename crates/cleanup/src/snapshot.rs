//! Знімки розмірів і «Що виросло»: порівняння з минулим сканом показує джерело проблеми.
//!
//! Знімок робиться після кожного скану (автоматичних фонових сканів немає), тому «за 7 днів»
//! означає «порівняно зі сканом приблизно тижневої давності»; фактичний період завжди показується.

use crate::fs::FsApi;
use crate::path;
use crate::CleanupError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Скільки знімків зберігати на диск.
pub const MAX_SNAPSHOTS: usize = 40;
/// Знімки, старші за це, видаляються (секунди).
pub const MAX_AGE_SECS: i64 = 90 * 86_400;
/// Ріст від цього розміру показується.
pub const GROWTH_MIN_BYTES: i64 = 50 * 1024 * 1024;
/// Базовий знімок не новіший за це (секунди): порівнювати з хвилинною давниною немає сенсу.
pub const MIN_BASELINE_AGE_SECS: i64 = 3600;

/// Знімок.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Час (Unix-секунди).
    pub unix: i64,
    /// Повний скан (інакше — лише корені правил).
    pub full: bool,
    /// Шлях → байти.
    pub sizes: BTreeMap<String, u64>,
}

/// Зміна розміру.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Growth {
    /// Шлях.
    pub path: String,
    /// Було (`None` — раніше менше за поріг знімка, тобто «з'явилося»).
    pub before: Option<u64>,
    /// Стало.
    pub after: u64,
    /// Різниця.
    pub delta: i64,
}

/// Що виросло.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GrowthReport {
    /// Базовий знімок (Unix-секунди).
    pub since_unix: i64,
    /// Поточний знімок (Unix-секунди).
    pub now_unix: i64,
    /// Обидва знімки повні (інакше видно лише корені правил).
    pub full: bool,
    /// Найбільший ріст першим.
    pub items: Vec<Growth>,
}

/// Вибирає базовий знімок: найновіший, старший за `target_age`; якщо такого немає — найстаріший,
/// але не новіший за [`MIN_BASELINE_AGE_SECS`].
pub fn pick_baseline(history: &[Snapshot], now_unix: i64, target_age: i64) -> Option<&Snapshot> {
    let eligible = history
        .iter()
        .filter(|s| now_unix - s.unix >= MIN_BASELINE_AGE_SECS);
    eligible
        .clone()
        .filter(|s| now_unix - s.unix >= target_age)
        .max_by_key(|s| s.unix)
        .or_else(|| eligible.min_by_key(|s| s.unix))
}

/// Порівнює знімки. Чиста функція.
///
/// Батьківська тека не показується, якщо її ріст майже весь (≥ 80%) пояснює показаний нащадок —
/// щоб у списку було джерело, а не всі його предки.
pub fn growth(base: &Snapshot, current: &Snapshot) -> GrowthReport {
    let full = base.full && current.full;
    let mut items: Vec<Growth> = current
        .sizes
        .iter()
        .filter_map(|(p, &after)| {
            let before = base.sizes.get(p).copied();
            // Тека, якої немає в частковому знімку, — невідомо, тож не вигадуємо «з'явилося».
            if before.is_none() && !base.full {
                return None;
            }
            let delta = i64::try_from(after).ok()? - i64::try_from(before.unwrap_or(0)).ok()?;
            (delta >= GROWTH_MIN_BYTES).then(|| Growth {
                path: p.clone(),
                before,
                after,
                delta,
            })
        })
        .collect();
    items.sort_by_key(|g| std::cmp::Reverse(g.delta));
    let snapshot = items.clone();
    items.retain(|g| {
        !snapshot.iter().any(|child| {
            path::is_strictly_within(&child.path, &g.path) && child.delta * 10 >= g.delta * 8
        })
    });
    GrowthReport {
        since_unix: base.unix,
        now_unix: current.unix,
        full,
        items,
    }
}

/// Знімки одного диска в JSON-файлі.
pub struct SnapshotStore<'a> {
    fs: &'a dyn FsApi,
    path: String,
}

impl<'a> SnapshotStore<'a> {
    /// Сховище: `dir\snapshots-<літера>.json`.
    pub fn new(fs: &'a dyn FsApi, dir: &str, drive: char) -> Self {
        Self {
            fs,
            path: path::join(
                dir,
                &format!("snapshots-{}.json", drive.to_ascii_uppercase()),
            ),
        }
    }

    /// Усі знімки (найстаріші першими).
    pub fn load(&self) -> Result<Vec<Snapshot>, CleanupError> {
        match self.fs.read_text(&self.path).map_err(CleanupError::Fs)? {
            Some(t) if !t.trim().is_empty() => Ok(serde_json::from_str(&t)?),
            _ => Ok(Vec::new()),
        }
    }

    /// Додає знімок, прибираючи застарілі.
    pub fn push(&self, snap: Snapshot) -> Result<(), CleanupError> {
        let now = snap.unix;
        let mut all = self.load()?;
        all.push(snap);
        all.retain(|s| now - s.unix <= MAX_AGE_SECS);
        all.sort_by_key(|s| s.unix);
        let start = all.len().saturating_sub(MAX_SNAPSHOTS);
        let json = serde_json::to_string(&all[start..])?;
        self.fs
            .write_text_atomic(&self.path, &json)
            .map_err(CleanupError::Fs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::MemFs;

    const MB: u64 = 1024 * 1024;
    const DAY: i64 = 86_400;

    fn snap(unix: i64, full: bool, entries: &[(&str, u64)]) -> Snapshot {
        Snapshot {
            unix,
            full,
            sizes: entries
                .iter()
                .map(|(p, b)| ((*p).to_string(), *b))
                .collect(),
        }
    }

    #[test]
    fn shows_source_not_ancestors() {
        let base = snap(
            0,
            true,
            &[(r"C:\Users", 1000 * MB), (r"C:\Users\Me\AppData", 500 * MB)],
        );
        let cur = snap(
            7 * DAY,
            true,
            &[
                (r"C:\Users", 2000 * MB),
                (r"C:\Users\Me\AppData", 1450 * MB),
                (r"C:\Games\New", 300 * MB),
            ],
        );
        let r = growth(&base, &cur);
        let paths: Vec<_> = r.items.iter().map(|g| g.path.as_str()).collect();
        assert_eq!(paths, vec![r"C:\Users\Me\AppData", r"C:\Games\New"]);
        assert_eq!(r.items[1].before, None);
    }

    #[test]
    fn partial_baseline_does_not_invent_new_items() {
        let base = snap(0, false, &[(r"C:\Temp", 10 * MB)]);
        let cur = snap(
            DAY,
            true,
            &[(r"C:\Temp", 200 * MB), (r"C:\Other", 900 * MB)],
        );
        let r = growth(&base, &cur);
        assert_eq!(r.items.len(), 1);
        assert!(!r.full);
    }

    #[test]
    fn baseline_choice() {
        let h = vec![
            snap(0, true, &[]),
            snap(5 * DAY, true, &[]),
            snap(9 * DAY, true, &[]),
        ];
        let now = 13 * DAY;
        assert_eq!(pick_baseline(&h, now, 7 * DAY).unwrap().unix, 5 * DAY);
        assert_eq!(pick_baseline(&h, 9 * DAY + 60, 7 * DAY).unwrap().unix, 0);
        assert!(pick_baseline(&h[..1], 30, 7 * DAY).is_none());
    }

    #[test]
    fn store_keeps_recent() {
        let fs = MemFs::new();
        let st = SnapshotStore::new(&fs, r"C:\data", 'c');
        st.push(snap(0, true, &[(r"C:\a", 1)])).unwrap();
        st.push(snap(100 * DAY, true, &[(r"C:\a", 2)])).unwrap();
        let all = st.load().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].unix, 100 * DAY);
    }
}

//! Сесії чорної скриньки: фризи гри за сесію режиму гри з можливими причинами і знімки
//! навколо найгірших фризів. Файли — `<id>.json` і `<id>.<n>.rec.gz` в окремій теці (байти
//! знімків дає застосунок).

use crate::model::SystemState;
use crate::store::{checked, Files, HistoryError};
use serde::{Deserialize, Serialize};

/// Скільки сесій тримати (старіші видаляються разом зі знімками).
pub const KEEP_SESSIONS: usize = 10;
/// Скільки знімків найгірших фризів лишати на сесію.
pub const KEEP_FREEZE_RECORDINGS: usize = 20;
/// Код причини, коли збігу ні з чим немає.
pub const UNKNOWN_CAUSE: &str = "unknown";

const ENTRY_EXT: &str = ".json";
const REC_EXT: &str = ".rec.gz";

/// Один фриз.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FreezeEntry {
    /// Unix-час кадру, мс.
    pub at_unix_ms: Option<i64>,
    /// Тривалість кадру, мс.
    pub frame_ms: f64,
    /// Коди ознак поруч (`shader`, `disk`, `dpc`…), конкретніші першими; порожньо — невідомо.
    pub causes: Vec<String>,
    /// Номер знімка навколо фризу (`None` — знімка немає або його прибрано).
    pub recording: Option<u32>,
    /// Час фризу від початку знімка, мс.
    pub t_ms: f64,
}

/// Сесія чорної скриньки.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SessionEntry {
    /// Ідентифікатор ([`crate::entry_id`] від гри й початку).
    pub id: String,
    /// Ім'я exe гри.
    pub game: String,
    /// Початок, Unix мс.
    pub started_unix_ms: i64,
    /// Кінець, Unix мс.
    pub ended_unix_ms: i64,
    /// Скільки секунд гри зараховано (у фокусі, після прогріву).
    pub played_s: f64,
    /// Скільки секунд скринька стояла на паузі (запис «Діагностики», A/B…).
    pub paused_s: f64,
    /// Поріг фризу, мс.
    pub threshold_ms: f64,
    /// Фризи за часом.
    pub freezes: Vec<FreezeEntry>,
    /// Стан ПК на початку сесії.
    pub state: SystemState,
}

/// Скільки фризів з такою головною причиною.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FreezeCauseCount {
    /// Код причини або [`UNKNOWN_CAUSE`].
    pub cause: String,
    /// Кількість фризів.
    pub count: u32,
}

/// Зведення за головною (першою) причиною фризу, найчастіші першими; «невідомо» — останнім.
pub fn cause_summary(s: &SessionEntry) -> Vec<FreezeCauseCount> {
    let mut out: Vec<FreezeCauseCount> = Vec::new();
    for f in &s.freezes {
        let cause = f.causes.first().map_or(UNKNOWN_CAUSE, String::as_str);
        match out.iter_mut().find(|c| c.cause == cause) {
            Some(c) => c.count += 1,
            None => out.push(FreezeCauseCount {
                cause: cause.to_string(),
                count: 1,
            }),
        }
    }
    // Стабільне сортування: за рівних — у порядку першої появи.
    out.sort_by_key(|c| (c.cause == UNKNOWN_CAUSE, std::cmp::Reverse(c.count)));
    out
}

/// Лишає знімки лише навколо `keep` найдовших фризів (знімок може містити кілька фризів).
/// Повертає номери знімків, які треба видалити; у фризів без знімка `recording` стає `None`.
pub fn keep_worst(s: &mut SessionEntry, keep: usize) -> Vec<u32> {
    let mut order: Vec<usize> = (0..s.freezes.len()).collect();
    order.sort_by(|&a, &b| s.freezes[b].frame_ms.total_cmp(&s.freezes[a].frame_ms));
    let mut kept: Vec<u32> = Vec::new();
    for i in order {
        if let Some(n) = s.freezes[i].recording {
            if !kept.contains(&n) && kept.len() < keep {
                kept.push(n);
            }
        }
    }
    let mut removed: Vec<u32> = Vec::new();
    for f in &mut s.freezes {
        if let Some(n) = f.recording.filter(|n| !kept.contains(n)) {
            if !removed.contains(&n) {
                removed.push(n);
            }
            f.recording = None;
        }
    }
    removed.sort_unstable();
    removed
}

/// Сховище сесій.
pub struct SessionStore<'a> {
    files: &'a dyn Files,
}

impl<'a> SessionStore<'a> {
    /// Сесії в цих файлах.
    pub fn new(files: &'a dyn Files) -> Self {
        Self { files }
    }

    /// Зберігає (замінює) підсумок сесії.
    pub fn save(&self, s: &SessionEntry) -> Result<(), HistoryError> {
        let id = checked(&s.id)?;
        self.files
            .write(&format!("{id}{ENTRY_EXT}"), &serde_json::to_vec_pretty(s)?)
    }

    /// Зберігає знімок `n` сесії `id`.
    pub fn save_recording(&self, id: &str, n: u32, bytes: &[u8]) -> Result<(), HistoryError> {
        let id = checked(id)?;
        self.files.write(&format!("{id}.{n}{REC_EXT}"), bytes)
    }

    /// Знімок `n` сесії `id`.
    pub fn recording(&self, id: &str, n: u32) -> Result<Option<Vec<u8>>, HistoryError> {
        let id = checked(id)?;
        self.files.read(&format!("{id}.{n}{REC_EXT}"))
    }

    /// Видаляє знімок `n` сесії `id`.
    pub fn remove_recording(&self, id: &str, n: u32) -> Result<(), HistoryError> {
        let id = checked(id)?;
        self.files.remove(&format!("{id}.{n}{REC_EXT}"))
    }

    /// Усі сесії, найновіші першими, і скільки файлів не вдалося прочитати.
    pub fn list(&self) -> Result<(Vec<SessionEntry>, usize), HistoryError> {
        let mut out = Vec::new();
        let mut broken = 0;
        for name in self.files.list()? {
            let Some(id) = name.strip_suffix(ENTRY_EXT) else {
                continue;
            };
            if checked(id).is_err() {
                continue;
            }
            match self
                .files
                .read(&name)?
                .map(|b| serde_json::from_slice::<SessionEntry>(&b))
            {
                Some(Ok(e)) => out.push(e),
                Some(Err(_)) => broken += 1,
                None => {}
            }
        }
        out.sort_by(|a, b| {
            b.started_unix_ms
                .cmp(&a.started_unix_ms)
                .then(b.id.cmp(&a.id))
        });
        Ok((out, broken))
    }

    /// Видаляє сесію разом з усіма її знімками.
    pub fn delete(&self, id: &str) -> Result<(), HistoryError> {
        let id = checked(id)?;
        let prefix = format!("{id}.");
        for name in self.files.list()? {
            let rec = name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(REC_EXT))
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty());
            if rec {
                self.files.remove(&name)?;
            }
        }
        self.files.remove(&format!("{id}{ENTRY_EXT}"))
    }

    /// Лишає `keep` найновіших сесій; повертає, скільки видалено.
    pub fn prune(&self, keep: usize) -> Result<usize, HistoryError> {
        let (sessions, _) = self.list()?;
        let mut removed = 0;
        for s in sessions.iter().skip(keep) {
            self.delete(&s.id)?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemFiles;

    fn freeze(ms: f64, causes: &[&str], recording: Option<u32>) -> FreezeEntry {
        FreezeEntry {
            at_unix_ms: Some(1),
            frame_ms: ms,
            causes: causes.iter().map(|c| c.to_string()).collect(),
            recording,
            t_ms: 1000.0,
        }
    }

    fn session(start: i64, freezes: Vec<FreezeEntry>) -> SessionEntry {
        SessionEntry {
            id: crate::entry_id("re9.exe", start),
            game: "re9.exe".into(),
            started_unix_ms: start,
            ended_unix_ms: start + 1000,
            played_s: 3600.0,
            paused_s: 0.0,
            threshold_ms: 100.0,
            freezes,
            state: SystemState::default(),
        }
    }

    #[test]
    fn summary_counts_main_cause_unknown_last() {
        let s = session(
            1,
            vec![
                freeze(120.0, &[], None),
                freeze(150.0, &["shader", "disk"], None),
                freeze(200.0, &["disk"], None),
                freeze(110.0, &["shader"], None),
                freeze(300.0, &[], None),
                freeze(130.0, &[], None),
            ],
        );
        let c = cause_summary(&s);
        let pairs: Vec<(&str, u32)> = c.iter().map(|c| (c.cause.as_str(), c.count)).collect();
        assert_eq!(pairs, vec![("shader", 2), ("disk", 1), ("unknown", 3)]);
    }

    #[test]
    fn keeps_recordings_of_the_worst_freezes() {
        let mut s = session(
            1,
            vec![
                freeze(120.0, &[], Some(1)),
                freeze(500.0, &[], Some(2)),
                freeze(400.0, &[], Some(2)), // той самий знімок
                freeze(300.0, &[], Some(3)),
                freeze(900.0, &[], None),
            ],
        );
        assert_eq!(keep_worst(&mut s, 2), vec![1]);
        let recs: Vec<Option<u32>> = s.freezes.iter().map(|f| f.recording).collect();
        assert_eq!(recs, vec![None, Some(2), Some(2), Some(3), None]);
    }

    #[test]
    fn store_saves_lists_deletes_with_recordings_and_prunes() {
        let files = MemFiles::default();
        let store = SessionStore::new(&files);
        let old = session(1, vec![freeze(120.0, &[], Some(1))]);
        let new = session(2, vec![]);
        store.save(&old).unwrap();
        store.save_recording(&old.id, 1, b"rec").unwrap();
        store.save(&new).unwrap();
        // Чужий файл з подібним ім'ям не чіпаємо.
        files.write(&format!("{}.x.rec.gz", old.id), b"?").unwrap();
        let (list, broken) = store.list().unwrap();
        assert_eq!((list.len(), broken), (2, 0));
        assert_eq!(list[0].id, new.id);
        assert_eq!(
            store.recording(&old.id, 1).unwrap().as_deref(),
            Some(&b"rec"[..])
        );
        assert_eq!(store.prune(1).unwrap(), 1);
        assert_eq!(store.recording(&old.id, 1).unwrap(), None);
        let names = files.list().unwrap();
        assert!(names.contains(&format!("{}.x.rec.gz", old.id)));
        assert!(!names.contains(&format!("{}.json", old.id)));
        assert!(store.save_recording("../x", 1, b"").is_err());
    }
}

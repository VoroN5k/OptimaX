//! Один кадр із CSV PresentMon.

use serde::{Deserialize, Serialize};

/// Момент виклику Present().
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum FrameTimestamp {
    /// Тики QPC (`--qpc_time`, колонка `TimeInQPC`) — та сама шкала, що й метрики.
    Qpc(i64),
    /// Мілісекунди від старту запису PresentMon (`TimeInMs` / `TimeInSeconds`).
    Ms(f64),
}

/// Кадр.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameRecord {
    /// Ім'я процесу.
    pub application: String,
    /// PID.
    pub process_id: u32,
    /// Адреса swap chain (кілька в одного процесу — напр. лаунчер і гра).
    pub swap_chain: String,
    /// Режим показу (`Hardware: Independent Flip`, `Composed: Flip` …).
    pub present_mode: String,
    /// Час кадру.
    pub time: FrameTimestamp,
    /// Час між цим і попереднім Present(), мс — це і є frametime.
    pub ms_between_presents: f64,
    /// Скільки попередній кадр був на екрані, мс (`None` — кадр не показано).
    pub ms_between_display_change: Option<f64>,
    /// Скільки CPU працював над кадром, мс (PresentMon 2.x).
    pub ms_cpu_busy: Option<f64>,
    /// Скільки GPU працював над кадром, мс (PresentMon 2.x).
    pub ms_gpu_busy: Option<f64>,
}

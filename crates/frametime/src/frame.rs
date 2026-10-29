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
    /// Синхроінтервал, який запросила гра (`SyncInterval`; 0 — без очікування vsync).
    pub sync_interval: Option<i32>,
    /// Чи дозволено розриви кадру (`AllowsTearing`).
    pub allows_tearing: Option<bool>,
}

/// Шлях кадру на екран — значення колонки `PresentMode`.
///
/// Рядки — дослівно з `PresentModeToString` у PresentMon 2.6.0 (`PresentMon/CsvOutput.cpp`,
/// тег `v2.6.0`), опис — з таблиці PresentMode у `README-ConsoleApplication.md` того ж тегу.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentMode {
    /// Гра заволоділа екраном і щокадру міняє поверхню на екрані.
    HardwareLegacyFlip,
    /// Гра заволоділа екраном і копіює кадр у поверхню, що вже на екрані.
    HardwareLegacyCopy,
    /// Екран не у власності гри, але поверхня на екрані міняється щокадру (без DWM).
    HardwareIndependentFlip,
    /// Flip-модель, гра отримала апаратну площину оверлея (без DWM).
    HardwareComposedIndependentFlip,
    /// Вікно з flip-моделлю: поверхні спільні з DWM, кадр складає DWM.
    ComposedFlip,
    /// Вікно, кадр копіюється в поверхню, спільну з GDI (складає DWM).
    ComposedCopyGpuGdi,
    /// Вікно, кадр копіюється в окрему поверхню DirectX, з GDI його складає DWM.
    ComposedCopyCpuGdi,
    /// `Other`, порожньо (колонку вимкнено) або невідомий рядок.
    Other,
}

impl PresentMode {
    pub(crate) const NAMES: [(Self, &'static str); 7] = [
        (Self::HardwareLegacyFlip, "Hardware: Legacy Flip"),
        (
            Self::HardwareLegacyCopy,
            "Hardware: Legacy Copy to front buffer",
        ),
        (Self::HardwareIndependentFlip, "Hardware: Independent Flip"),
        (
            Self::HardwareComposedIndependentFlip,
            "Hardware Composed: Independent Flip",
        ),
        (Self::ComposedFlip, "Composed: Flip"),
        (Self::ComposedCopyGpuGdi, "Composed: Copy with GPU GDI"),
        (Self::ComposedCopyCpuGdi, "Composed: Copy with CPU GDI"),
    ];

    /// Розпізнає рядок PresentMon (точний збіг, без урахування регістру й пробілів по краях).
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        Self::NAMES
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(raw))
            .map_or(Self::Other, |(m, _)| *m)
    }

    /// Рядок, як його пише PresentMon.
    pub fn as_str(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(m, _)| *m == self)
            .map_or("Other", |(_, n)| n)
    }

    /// `Some(true)` — кадр складає DWM, `Some(false)` — показ напряму, `None` — невідомо.
    pub fn is_composed(self) -> Option<bool> {
        match self {
            Self::ComposedFlip | Self::ComposedCopyGpuGdi | Self::ComposedCopyCpuGdi => Some(true),
            Self::HardwareLegacyFlip
            | Self::HardwareLegacyCopy
            | Self::HardwareIndependentFlip
            | Self::HardwareComposedIndependentFlip => Some(false),
            Self::Other => None,
        }
    }
}

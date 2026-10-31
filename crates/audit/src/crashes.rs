//! «Чому вилітає?» — раптові вимкнення, сині екрани й вильоти програм із журналу подій.
//!
//! Лише чисті функції над уже прочитаними подіями ([`SystemEvent`]), тож усе покривається
//! тестами. Факти з журналу (що, коли, в якому модулі) — з високою впевненістю; причина —
//! лише «найчастіша для такого випадку», з помірною впевненістю й порадами як перевірки.
//!
//! Довідники:
//! - коди синіх екранів — Microsoft «Bug Check Code Reference»
//!   (learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-code-reference2);
//! - коди винятків — стандартні NTSTATUS (`ntstatus.h`) і коди винятків C++ / .NET;
//! - модулі — відомі імена бібліотек драйверів відеокарт, DirectX і оверлеїв.
//!
//! Утримання кнопки живлення вимикає ПК апаратно, повз Windows: для журналу це те саме, що
//! зникнення живлення (`PowerButtonTimestamp` = 0). Тому розрізнити їх неможливо — для цього
//! є прапорець [`CrashContext::hard_off_by_button`].

use crate::model::SystemEvent;
use crate::rules::{
    whea_kind, WheaSeverity, APP_ERROR_EVENT_ID, APP_ERROR_PROVIDER, DISPLAY_PROVIDER,
    KERNEL_POWER_PROVIDER, TDR_EVENT_ID, UNEXPECTED_OFF_EVENT_ID, WHEA_PROVIDER,
};
use optimax_core::{Category, CivilDate, Finding, FindingBuilder, Impact, Text};
use serde::Serialize;
use std::collections::BTreeMap;

/// За скільки днів аналізуємо журнал.
pub const CRASH_DAYS: u32 = 90;
/// Стільки вильотів однієї програми — закономірність, а не випадковість.
pub const APP_FREQUENT: usize = 5;
/// Стільки раптових вимкнень за 30 днів (без «вимикаю кнопкою») — серйозний сигнал.
pub const POWER_FREQUENT_30D: usize = 3;
/// Не більше записів у хронології.
pub const TIMELINE_MAX: usize = 300;

fn draft(id: &'static str, impact: Impact, confidence: u8) -> FindingBuilder {
    FindingBuilder::new("crash", id, Category::Crashes, impact, confidence)
}

fn num(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

// ------------------------------------------------------------------------------------------
// Розбір подій

/// Раптове вимкнення (Kernel-Power 41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerLoss {
    /// Час, Unix-секунди.
    pub unix_s: i64,
    /// Дата.
    pub date: CivilDate,
    /// Код синього екрана (0 — синього екрана не було).
    pub bugcheck: u32,
    /// Натискали кнопку живлення (Windows встигла це зафіксувати).
    pub power_button: bool,
    /// Під час переходу в сон чи виходу з нього.
    pub sleep: bool,
}

/// Виліт програми (Application Error 1000).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppCrash {
    /// Час, Unix-секунди.
    pub unix_s: i64,
    /// Дата.
    pub date: CivilDate,
    /// Ім'я exe.
    pub app: String,
    /// Модуль, у якому стався збій.
    pub module: String,
    /// Код винятку.
    pub exception: u32,
    /// Шлях до exe.
    pub app_path: String,
    /// Шлях до модуля.
    pub module_path: String,
}

/// Розбирає Kernel-Power 41.
pub fn power_event(e: &SystemEvent) -> Option<PowerLoss> {
    if !e.provider.eq_ignore_ascii_case(KERNEL_POWER_PROVIDER) || e.id != UNEXPECTED_OFF_EVENT_ID {
        return None;
    }
    let field = |i: usize| e.data.get(i).and_then(|s| num(s));
    Some(PowerLoss {
        unix_s: e.unix_s,
        date: e.date,
        bugcheck: u32::try_from(field(0)?).ok()?,
        sleep: field(5).is_some_and(|v| v != 0),
        power_button: field(6).is_some_and(|v| v != 0),
    })
}

/// Розбирає Application Error 1000.
pub fn app_event(e: &SystemEvent) -> Option<AppCrash> {
    if !e.provider.eq_ignore_ascii_case(APP_ERROR_PROVIDER) || e.id != APP_ERROR_EVENT_ID {
        return None;
    }
    let get = |i: usize| e.data.get(i).cloned().unwrap_or_default();
    let app = get(0);
    if app.is_empty() {
        return None;
    }
    // Код у цій події — hex без префікса (напр. `c0000374`).
    let exception = u32::from_str_radix(get(6).trim().trim_start_matches("0x"), 16).ok()?;
    Some(AppCrash {
        unix_s: e.unix_s,
        date: e.date,
        app,
        module: get(3),
        exception,
        app_path: get(10),
        module_path: get(11),
    })
}

// ------------------------------------------------------------------------------------------
// Довідники

/// Чого стосується синій екран.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BugcheckGroup {
    /// Драйвер звернувся не туди або завис.
    Driver,
    /// Оперативна пам'ять або керування нею.
    Memory,
    /// Диск або читання з нього.
    Storage,
    /// Відеокарта або її драйвер.
    Gpu,
    /// Апаратна помилка (процесор, шина, розгін).
    Hardware,
    /// Критичний процес Windows.
    System,
}

impl BugcheckGroup {
    fn key(self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Memory => "memory",
            Self::Storage => "storage",
            Self::Gpu => "gpu",
            Self::Hardware => "hardware",
            Self::System => "system",
        }
    }
}

/// Назва й група коду синього екрана (найчастіші коди; невідомі — `None`).
pub fn bugcheck(code: u32) -> Option<(&'static str, BugcheckGroup)> {
    use BugcheckGroup::*;
    Some(match code {
        0x0A => ("IRQL_NOT_LESS_OR_EQUAL", Driver),
        0x19 => ("BAD_POOL_HEADER", Memory),
        0x1A => ("MEMORY_MANAGEMENT", Memory),
        0x1E => ("KMODE_EXCEPTION_NOT_HANDLED", Driver),
        0x3B => ("SYSTEM_SERVICE_EXCEPTION", Driver),
        0x50 => ("PAGE_FAULT_IN_NONPAGED_AREA", Memory),
        0x7A => ("KERNEL_DATA_INPAGE_ERROR", Storage),
        0x7E => ("SYSTEM_THREAD_EXCEPTION_NOT_HANDLED", Driver),
        0x9F => ("DRIVER_POWER_STATE_FAILURE", Driver),
        0xC2 => ("BAD_POOL_CALLER", Driver),
        0xD1 => ("DRIVER_IRQL_NOT_LESS_OR_EQUAL", Driver),
        0xEF => ("CRITICAL_PROCESS_DIED", System),
        0xF4 => ("CRITICAL_OBJECT_TERMINATION", Storage),
        0x101 => ("CLOCK_WATCHDOG_TIMEOUT", Hardware),
        0x116 => ("VIDEO_TDR_FAILURE", Gpu),
        0x117 => ("VIDEO_TDR_TIMEOUT_DETECTED", Gpu),
        0x119 => ("VIDEO_SCHEDULER_INTERNAL_ERROR", Gpu),
        0x124 => ("WHEA_UNCORRECTABLE_ERROR", Hardware),
        0x133 => ("DPC_WATCHDOG_VIOLATION", Driver),
        0x139 => ("KERNEL_SECURITY_CHECK_FAILURE", Driver),
        0x154 => ("UNEXPECTED_STORE_EXCEPTION", Storage),
        _ => return None,
    })
}

/// Що за помилка у програмі (за кодом винятку).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    /// `c0000005` — звернення до чужої або звільненої пам'яті.
    AccessViolation,
    /// `c0000374` — пошкоджено купу (структури виділеної пам'яті).
    HeapCorruption,
    /// `c0000409` — аварійне завершення програмою самою (fail fast / переповнення буфера).
    FailFast,
    /// `c00000fd` — переповнення стеку.
    StackOverflow,
    /// `c000001d` — недопустима інструкція процесора.
    IllegalInstruction,
    /// `c0000135` / `c0000142` — не знайдено або не завантажилась бібліотека.
    DllProblem,
    /// `e06d7363` — неперехоплений виняток C++.
    CppException,
    /// `e0434352` — неперехоплений виняток .NET.
    DotNetException,
    /// `80000003` — точка зупину (програма зупинила себе навмисно).
    Breakpoint,
    /// Інший код.
    Other,
}

impl ExceptionKind {
    fn key(self) -> &'static str {
        match self {
            Self::AccessViolation => "access_violation",
            Self::HeapCorruption => "heap",
            Self::FailFast => "fail_fast",
            Self::StackOverflow => "stack_overflow",
            Self::IllegalInstruction => "illegal_instruction",
            Self::DllProblem => "dll",
            Self::CppException => "cpp",
            Self::DotNetException => "dotnet",
            Self::Breakpoint => "breakpoint",
            Self::Other => "other",
        }
    }

    /// Помилка роботи з пам'яттю (може бути наслідком нестабільної RAM).
    fn memory_related(self) -> bool {
        matches!(self, Self::AccessViolation | Self::HeapCorruption)
    }
}

/// Різновид винятку за кодом.
pub fn exception_kind(code: u32) -> ExceptionKind {
    match code {
        0xC000_0005 => ExceptionKind::AccessViolation,
        0xC000_0374 => ExceptionKind::HeapCorruption,
        0xC000_0409 => ExceptionKind::FailFast,
        0xC000_00FD => ExceptionKind::StackOverflow,
        0xC000_001D => ExceptionKind::IllegalInstruction,
        0xC000_0135 | 0xC000_0142 => ExceptionKind::DllProblem,
        0xE06D_7363 => ExceptionKind::CppException,
        0xE043_4352 => ExceptionKind::DotNetException,
        0x8000_0003 => ExceptionKind::Breakpoint,
        _ => ExceptionKind::Other,
    }
}

/// Де стався збій.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// Сама програма або бібліотека з її теки.
    App,
    /// Драйвер відеокарти NVIDIA.
    GpuNvidia,
    /// Драйвер відеокарти AMD.
    GpuAmd,
    /// Драйвер відеокарти Intel.
    GpuIntel,
    /// DirectX / Vulkan / OpenGL (системна частина графіки).
    Graphics,
    /// Оверлей (Discord, Steam, RivaTuner).
    Overlay,
    /// Системна бібліотека Windows: програма сама кинула виняток через неї.
    SystemLib,
    /// Бібліотека з іншого місця (мод, плагін, стороння програма).
    ThirdParty,
    /// Компонент Windows (бібліотека з теки Windows, якої немає в списках вище).
    WindowsComponent,
    /// Windows не змогла визначити модуль.
    Unknown,
}

impl ModuleKind {
    fn key(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::GpuNvidia => "gpu_nvidia",
            Self::GpuAmd => "gpu_amd",
            Self::GpuIntel => "gpu_intel",
            Self::Graphics => "graphics",
            Self::Overlay => "overlay",
            Self::SystemLib => "system_lib",
            Self::ThirdParty => "third_party",
            Self::WindowsComponent => "windows",
            Self::Unknown => "unknown",
        }
    }

    fn gpu(self) -> bool {
        matches!(self, Self::GpuNvidia | Self::GpuAmd | Self::GpuIntel)
    }
}

/// Бібліотеки користувацького режиму драйверів відеокарт (за початком імені).
const NVIDIA_MODULES: &[&str] = &[
    "nvwgf2um",
    "nvoglv",
    "nvd3dum",
    "nvldumd",
    "nvgpucomp",
    "nvapi",
];
const AMD_MODULES: &[&str] = &["amdxx", "atidxx", "atiumd", "amdvlk", "atio6a", "amdxc"];
const INTEL_MODULES: &[&str] = &["igd10", "igd11", "igd12", "igdumd", "igxelp", "igvk"];
/// Системні частини графіки.
const GRAPHICS_MODULES: &[&str] = &[
    "d3d9.dll",
    "d3d10.dll",
    "d3d11.dll",
    "d3d12.dll",
    "d3d12core.dll",
    "dxgi.dll",
    "dxcore.dll",
    "vulkan-1.dll",
    "opengl32.dll",
];
/// Оверлеї, що впроваджуються в гру.
const OVERLAY_MODULES: &[&str] = &[
    "discordhook64.dll",
    "discordhook.dll",
    "gameoverlayrenderer64.dll",
    "gameoverlayrenderer.dll",
    "rtsshooks64.dll",
    "rtsshooks.dll",
];
/// Системні бібліотеки, через які програма кидає власні винятки.
const SYSTEM_MODULES: &[&str] = &[
    "ntdll.dll",
    "kernelbase.dll",
    "kernel32.dll",
    "ucrtbase.dll",
    "msvcrt.dll",
    "msvcp140.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "combase.dll",
];

fn dir_of(path: &str) -> Option<String> {
    let p = path.trim().to_lowercase();
    p.rfind('\\').map(|i| p[..i].to_string())
}

/// Шлях лежить у теці `dir` (обидва — у нижньому регістрі, без `\` у кінці).
fn under(path: &str, dir: &str) -> bool {
    !dir.is_empty()
        && path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('\\'))
}

/// Де стався збій — за іменем модуля і шляхами. `windows_dir` — тека Windows (`C:\Windows`).
pub fn module_kind(
    module: &str,
    module_path: &str,
    app: &str,
    app_path: &str,
    windows_dir: &str,
) -> ModuleKind {
    let m = module.trim().to_lowercase();
    let windows = windows_dir.trim().trim_end_matches('\\').to_lowercase();
    if m.is_empty() || m == "unknown" {
        return ModuleKind::Unknown;
    }
    let starts = |list: &[&str]| list.iter().any(|p| m.starts_with(p));
    if starts(NVIDIA_MODULES) {
        ModuleKind::GpuNvidia
    } else if starts(AMD_MODULES) {
        ModuleKind::GpuAmd
    } else if starts(INTEL_MODULES) {
        ModuleKind::GpuIntel
    } else if GRAPHICS_MODULES.contains(&m.as_str()) {
        ModuleKind::Graphics
    } else if OVERLAY_MODULES.contains(&m.as_str()) {
        ModuleKind::Overlay
    } else if SYSTEM_MODULES.contains(&m.as_str()) {
        ModuleKind::SystemLib
    } else if m == app.trim().to_lowercase()
        || dir_of(module_path).is_some_and(|d| Some(&d) == dir_of(app_path).as_ref())
    {
        ModuleKind::App
    } else if dir_of(module_path).is_some_and(|d| under(&d, &windows)) {
        ModuleKind::WindowsComponent
    } else if dir_of(app_path).is_some_and(|a| {
        // Підтека програми (плагіни, моди самої гри) — але не для програм із теки Windows:
        // інакше будь-яка бібліотека Windows стала б «своєю» для Explorer.
        !under(&a, &windows) && dir_of(module_path).is_some_and(|d| under(&d, &a))
    }) {
        ModuleKind::App
    } else {
        ModuleKind::ThirdParty
    }
}

// ------------------------------------------------------------------------------------------
// Звіт

/// Що враховує аналіз окрім журналу.
#[derive(Debug, Clone, Default)]
pub struct CrashContext {
    /// Користувач вимикає ПК утриманням кнопки — раптові вимкнення без синього екрана для
    /// нього звичайні.
    pub hard_off_by_button: bool,
    /// Поточний час, Unix-секунди (для «за останні 30 днів»).
    pub now_unix: i64,
    /// Тека Windows (`C:\Windows`): бібліотеки з неї — компоненти Windows, а процеси з неї
    /// не враховуються у вердикті про RAM.
    pub windows_dir: String,
}

/// Різновид запису хронології.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CrashEntryKind {
    /// Раптове вимкнення без синього екрана.
    PowerLoss,
    /// Синій екран.
    Bsod,
    /// Виліт програми.
    AppCrash,
    /// Скидання драйвера відеокарти.
    GpuReset,
    /// Фатальна апаратна помилка (WHEA).
    Hardware,
}

/// Запис хронології.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CrashEntry {
    /// Час, Unix-секунди.
    pub unix_s: i64,
    /// Різновид.
    pub kind: CrashEntryKind,
    /// Короткий опис.
    pub text: Text,
}

/// Результат аналізу журналу.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct CrashReport {
    /// Період, днів.
    pub days: u32,
    /// Раптових вимкнень без синього екрана.
    pub power_losses: u32,
    /// Синіх екранів.
    pub bsods: u32,
    /// Вильотів програм.
    pub app_crashes: u32,
    /// Скидань драйвера відеокарти.
    pub gpu_resets: u32,
    /// Вердикти: від найважливішого.
    pub findings: Vec<Finding>,
    /// Хронологія: від нових до старих.
    pub timeline: Vec<CrashEntry>,
}

fn hex(code: u32) -> String {
    format!("0x{code:08X}")
}

/// Аналізує події журналу (`events` — з будь-якого порядку).
pub fn analyze(events: &[SystemEvent], ctx: &CrashContext) -> CrashReport {
    let powers: Vec<PowerLoss> = events.iter().filter_map(power_event).collect();
    let apps: Vec<AppCrash> = events.iter().filter_map(app_event).collect();
    let tdrs: Vec<&SystemEvent> = events
        .iter()
        .filter(|e| e.provider.eq_ignore_ascii_case(DISPLAY_PROVIDER) && e.id == TDR_EVENT_ID)
        .collect();
    let fatal_whea: Vec<&SystemEvent> = events
        .iter()
        .filter(|e| e.provider.eq_ignore_ascii_case(WHEA_PROVIDER))
        .filter(|e| matches!(whea_kind(e.id), Some((WheaSeverity::Fatal, _))))
        .collect();
    let (losses, bsods): (Vec<&PowerLoss>, Vec<&PowerLoss>) =
        powers.iter().partition(|p| p.bugcheck == 0);

    let mut findings = Vec::new();
    findings.extend(power_finding(&losses, ctx));
    findings.extend(bsod_findings(&bsods, !fatal_whea.is_empty()));
    findings.extend(app_findings(&apps, !tdrs.is_empty(), ctx));
    findings.extend(memory_pattern(&apps, ctx));
    if findings.is_empty() {
        findings.push(
            draft("none", Impact::Info, 90)
                .arg("days", CRASH_DAYS)
                .build(),
        );
    }
    findings.sort_by(|x, y| {
        y.impact
            .cmp(&x.impact)
            .then(y.confidence.cmp(&x.confidence))
    });

    let mut timeline: Vec<CrashEntry> = Vec::new();
    for p in &powers {
        timeline.push(match bugcheck(p.bugcheck) {
            _ if p.bugcheck == 0 => CrashEntry {
                unix_s: p.unix_s,
                kind: CrashEntryKind::PowerLoss,
                text: Text::new(if p.sleep {
                    "crash.tl.power_sleep"
                } else {
                    "crash.tl.power"
                }),
            },
            Some((name, _)) => CrashEntry {
                unix_s: p.unix_s,
                kind: CrashEntryKind::Bsod,
                text: Text::new("crash.tl.bsod")
                    .arg("name", name)
                    .arg("code", hex(p.bugcheck)),
            },
            None => CrashEntry {
                unix_s: p.unix_s,
                kind: CrashEntryKind::Bsod,
                text: Text::new("crash.tl.bsod")
                    .arg("name", "?")
                    .arg("code", hex(p.bugcheck)),
            },
        });
    }
    for a in &apps {
        timeline.push(CrashEntry {
            unix_s: a.unix_s,
            kind: CrashEntryKind::AppCrash,
            text: Text::new("crash.tl.app")
                .arg("app", &a.app)
                .arg("module", &a.module)
                .arg("code", format!("{:08x}", a.exception)),
        });
    }
    for e in &tdrs {
        timeline.push(CrashEntry {
            unix_s: e.unix_s,
            kind: CrashEntryKind::GpuReset,
            text: Text::new("crash.tl.gpu_reset")
                .arg("driver", e.data.first().cloned().unwrap_or_default()),
        });
    }
    for e in &fatal_whea {
        timeline.push(CrashEntry {
            unix_s: e.unix_s,
            kind: CrashEntryKind::Hardware,
            text: Text::new("crash.tl.whea"),
        });
    }
    timeline.sort_by_key(|e| std::cmp::Reverse(e.unix_s));
    timeline.truncate(TIMELINE_MAX);

    CrashReport {
        days: CRASH_DAYS,
        power_losses: count(losses.len()),
        bsods: count(bsods.len()),
        app_crashes: count(apps.len()),
        gpu_resets: count(tdrs.len()),
        findings,
        timeline,
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn last_date(dates: impl Iterator<Item = CivilDate>) -> String {
    dates.max().map(|d| d.to_string()).unwrap_or_default()
}

fn power_finding(losses: &[&PowerLoss], ctx: &CrashContext) -> Option<Finding> {
    if losses.is_empty() {
        return None;
    }
    let n = losses.len();
    let recent = losses
        .iter()
        .filter(|p| ctx.now_unix - p.unix_s <= 30 * 86_400)
        .count();
    let button = losses.iter().filter(|p| p.power_button).count();
    let sleep = losses.iter().filter(|p| p.sleep).count();
    let last = last_date(losses.iter().map(|p| p.date));
    let mut b = if ctx.hard_off_by_button {
        draft("power_user", Impact::Info, 60)
    } else if recent >= POWER_FREQUENT_30D {
        draft("power", Impact::High, 90)
    } else {
        draft("power", Impact::Moderate, 90)
    };
    b = b
        .arg("count", n)
        .arg("days", CRASH_DAYS)
        .arg("recent", recent)
        .advice()
        .evidence(Text::new("crash.ev.count"), Text::raw(n.to_string()))
        .evidence(
            Text::new("crash.ev.last_30d"),
            Text::raw(recent.to_string()),
        )
        .evidence(Text::new("crash.ev.last"), Text::raw(last));
    if button > 0 {
        b = b.evidence(
            Text::new("crash.ev.power_button"),
            Text::raw(button.to_string()),
        );
    }
    if sleep > 0 {
        b = b.evidence(Text::new("crash.ev.sleep"), Text::raw(sleep.to_string()));
    }
    Some(b.build())
}

fn bsod_findings(bsods: &[&PowerLoss], fatal_whea: bool) -> Vec<Finding> {
    let mut by_code: BTreeMap<u32, Vec<&PowerLoss>> = BTreeMap::new();
    for b in bsods {
        by_code.entry(b.bugcheck).or_default().push(b);
    }
    let mut groups: Vec<(u32, Vec<&PowerLoss>)> = by_code.into_iter().collect();
    groups.sort_by(|x, y| y.1.len().cmp(&x.1.len()).then(x.0.cmp(&y.0)));
    groups
        .into_iter()
        .take(3)
        .map(|(code, list)| {
            let (name, group) = match bugcheck(code) {
                Some((name, group)) => (name, Some(group)),
                None => ("?", None),
            };
            let group_key = group.map_or("unknown", BugcheckGroup::key);
            let mut b = draft("bsod", Impact::High, 75)
                .arg("name", name)
                .arg("code", hex(code))
                .arg("count", list.len())
                .evidence(
                    Text::new("crash.ev.what"),
                    Text::new(format!("crash.bsod.group.{group_key}")),
                )
                .evidence(
                    Text::new("crash.ev.count"),
                    Text::raw(list.len().to_string()),
                )
                .evidence(
                    Text::new("crash.ev.last"),
                    Text::raw(last_date(list.iter().map(|p| p.date))),
                )
                .evidence(
                    Text::new("crash.ev.what_to_do"),
                    Text::new(format!("crash.bsod.advice.{group_key}")),
                );
            if group == Some(BugcheckGroup::Hardware) && fatal_whea {
                b = b.evidence(Text::new("crash.ev.whea"), Text::new("crash.ev.whea_value"));
            }
            b.build()
        })
        .collect()
}

/// Найчастіше значення.
fn most_common<T: Copy + Ord>(items: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: BTreeMap<T, usize> = BTreeMap::new();
    for i in items {
        *counts.entry(i).or_default() += 1;
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
}

/// Порада за місцем і характером збою.
fn app_advice(kind: ModuleKind, exc: ExceptionKind, gpu_resets: bool) -> &'static str {
    match (kind, exc) {
        (_, ExceptionKind::IllegalInstruction) => "crash.advice.cpu",
        (_, ExceptionKind::DllProblem) => "crash.advice.dll",
        (k, _) if k.gpu() && gpu_resets => "crash.advice.gpu_driver_tdr",
        (k, _) if k.gpu() => "crash.advice.gpu_driver",
        (ModuleKind::Graphics, _) => "crash.advice.graphics",
        (ModuleKind::Overlay, _) => "crash.advice.overlay",
        (ModuleKind::ThirdParty, _) => "crash.advice.third_party",
        (ModuleKind::WindowsComponent, _) => "crash.advice.windows",
        (ModuleKind::App | ModuleKind::Unknown, e) if e.memory_related() => {
            "crash.advice.game_memory"
        }
        _ => "crash.advice.app",
    }
}

fn app_findings(apps: &[AppCrash], gpu_resets: bool, ctx: &CrashContext) -> Vec<Finding> {
    let mut by_app: BTreeMap<String, Vec<&AppCrash>> = BTreeMap::new();
    for a in apps {
        by_app.entry(a.app.to_lowercase()).or_default().push(a);
    }
    let mut groups: Vec<Vec<&AppCrash>> = by_app.into_values().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    groups
        .into_iter()
        .take(5)
        .filter_map(|list| {
            let first = *list.first()?;
            let kinds: Vec<ModuleKind> = list
                .iter()
                .map(|a| {
                    module_kind(
                        &a.module,
                        &a.module_path,
                        &a.app,
                        &a.app_path,
                        &ctx.windows_dir,
                    )
                })
                .collect();
            let kind = most_common(kinds.iter().map(|k| k.key()))
                .and_then(|key| kinds.iter().find(|k| k.key() == key).copied())
                .unwrap_or(ModuleKind::Unknown);
            let module = most_common(
                list.iter()
                    .zip(&kinds)
                    .filter(|(_, k)| **k == kind)
                    .map(|(a, _)| a.module.as_str()),
            )
            .unwrap_or("?");
            let exc = most_common(list.iter().map(|a| a.exception)).unwrap_or(0);
            let exc_kind = exception_kind(exc);
            // «Впливає на звичні речі» — лише якщо вилітає часто саме зараз, а не колись.
            let recent = list
                .iter()
                .filter(|a| ctx.now_unix - a.unix_s <= 30 * 86_400)
                .count();
            let impact = if recent >= APP_FREQUENT {
                Impact::High
            } else {
                Impact::Moderate
            };
            Some(
                draft("app", impact, 70)
                    .arg("app", &first.app)
                    .arg("count", list.len())
                    .arg("days", CRASH_DAYS)
                    .evidence(
                        Text::new("crash.ev.last_30d"),
                        Text::raw(recent.to_string()),
                    )
                    .evidence(
                        Text::new("crash.ev.where"),
                        Text::new(format!("crash.where.{}", kind.key())).arg("module", module),
                    )
                    .evidence(
                        Text::new("crash.ev.what"),
                        Text::new(format!("crash.exc.{}", exc_kind.key()))
                            .arg("code", format!("{exc:08x}")),
                    )
                    .evidence(
                        Text::new("crash.ev.last"),
                        Text::raw(last_date(list.iter().map(|a| a.date))),
                    )
                    .evidence(
                        Text::new("crash.ev.what_to_do"),
                        Text::new(app_advice(kind, exc_kind, gpu_resets)),
                    )
                    .build(),
            )
        })
        .collect()
}

/// Кілька різних програм падають з помилками пам'яті у власному коді — спільна причина
/// може бути в RAM (нестабільний розгін / XMP), а не в кожній програмі окремо.
fn memory_pattern(apps: &[AppCrash], ctx: &CrashContext) -> Option<Finding> {
    let windows = ctx.windows_dir.trim_end_matches('\\').to_lowercase();
    let hits: Vec<&AppCrash> = apps
        .iter()
        .filter(|a| exception_kind(a.exception).memory_related())
        // Процеси Windows не рахуємо: їхні збої — не ознака RAM у програмах користувача.
        .filter(|a| !dir_of(&a.app_path).is_some_and(|d| under(&d, &windows)))
        .filter(|a| {
            matches!(
                module_kind(
                    &a.module,
                    &a.module_path,
                    &a.app,
                    &a.app_path,
                    &ctx.windows_dir
                ),
                ModuleKind::App | ModuleKind::Unknown
            )
        })
        .collect();
    let mut names: Vec<String> = hits.iter().map(|a| a.app.clone()).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by_key(|n| n.to_lowercase());
    if names.len() < 2 || hits.len() < 3 {
        return None;
    }
    Some(
        draft("ram_pattern", Impact::High, 55)
            .arg("apps", names.len())
            .arg("count", hits.len())
            .advice()
            .evidence(Text::new("crash.ev.apps"), Text::raw(names.join(", ")))
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_793_000_000;

    fn ev(provider: &str, id: u16, days_ago: i64, data: &[&str]) -> SystemEvent {
        SystemEvent {
            provider: provider.into(),
            id,
            level: 2,
            date: CivilDate::new(2026, 10, 1).unwrap(),
            unix_s: NOW - days_ago * DAY,
            data: data.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Kernel-Power 41 як на реальній системі (поля в порядку EventData).
    fn kp41(days_ago: i64, bugcheck: &str, sleep: &str, button: &str) -> SystemEvent {
        ev(
            KERNEL_POWER_PROVIDER,
            41,
            days_ago,
            &[
                bugcheck, "0x0", "0x0", "0x0", "0x0", sleep, button, "0", "0", "false",
            ],
        )
    }

    /// Application Error 1000 як на реальній системі.
    fn app1000(
        days_ago: i64,
        app: &str,
        module: &str,
        code: &str,
        module_path: &str,
    ) -> SystemEvent {
        let app_path = format!(r"C:\Games\{app}\{app}");
        ev(
            APP_ERROR_PROVIDER,
            1000,
            days_ago,
            &[
                app,
                "1.0",
                "0",
                module,
                "1.0",
                "0",
                code,
                "0x1",
                "0x1",
                "0x1",
                &app_path,
                module_path,
            ],
        )
    }

    const WIN: &str = r"C:\Windows";

    fn ctx() -> CrashContext {
        CrashContext {
            hard_off_by_button: false,
            now_unix: NOW,
            windows_dir: WIN.into(),
        }
    }

    fn find<'a>(r: &'a CrashReport, id: &str) -> Vec<&'a Finding> {
        r.findings.iter().filter(|f| f.id == id).collect()
    }

    #[test]
    fn parses_real_event_layouts() {
        let p = power_event(&kp41(1, "0", "0", "0")).unwrap();
        assert_eq!((p.bugcheck, p.sleep, p.power_button), (0, false, false));
        let p = power_event(&kp41(1, "278", "1", "131000000")).unwrap();
        assert_eq!((p.bugcheck, p.sleep, p.power_button), (0x116, true, true));
        let a = app_event(&app1000(1, "TslGame.exe", "unknown", "c0000374", "unknown")).unwrap();
        assert_eq!(a.exception, 0xC000_0374);
        assert_eq!(a.module, "unknown");
        assert!(power_event(&app1000(1, "x.exe", "y", "c0000005", "")).is_none());
    }

    #[test]
    fn modules_are_classified() {
        let app = r"C:\Games\PUBG\TslGame.exe";
        assert_eq!(
            module_kind(
                "nvwgf2umx.dll",
                r"C:\Windows\System32\DriverStore\nvwgf2umx.dll",
                "TslGame.exe",
                app,
                WIN
            ),
            ModuleKind::GpuNvidia
        );
        assert_eq!(
            module_kind("amdxx64.dll", "", "g.exe", app, WIN),
            ModuleKind::GpuAmd
        );
        assert_eq!(
            module_kind("d3d12.dll", "", "g.exe", app, WIN),
            ModuleKind::Graphics
        );
        assert_eq!(
            module_kind("DiscordHook64.dll", "", "g.exe", app, WIN),
            ModuleKind::Overlay
        );
        assert_eq!(
            module_kind("KERNELBASE.dll", "", "g.exe", app, WIN),
            ModuleKind::SystemLib
        );
        assert_eq!(
            module_kind("TslGame.exe", app, "TslGame.exe", app, WIN),
            ModuleKind::App
        );
        assert_eq!(
            module_kind(
                "engine.dll",
                r"C:\Games\PUBG\bin\engine.dll",
                "TslGame.exe",
                app,
                WIN
            ),
            ModuleKind::App
        );
        assert_eq!(
            module_kind("mod.dll", r"C:\Mods\mod.dll", "TslGame.exe", app, WIN),
            ModuleKind::ThirdParty
        );
        assert_eq!(
            module_kind("unknown", "unknown", "g.exe", app, WIN),
            ModuleKind::Unknown
        );
    }

    #[test]
    fn windows_components_are_not_the_programs_own_code() {
        // Explorer лежить у теці Windows: її бібліотеки — компоненти Windows, а не «свої».
        assert_eq!(
            module_kind(
                "Microsoft.UI.Xaml.dll",
                r"C:\WINDOWS\SystemApps\Microsoft.WindowsAppRuntime.CBS\Microsoft.UI.Xaml.dll",
                "Explorer.EXE",
                r"C:\WINDOWS\Explorer.EXE",
                WIN
            ),
            ModuleKind::WindowsComponent
        );
        // Системний процес із помилкою пам'яті не рахується у вердикті про RAM.
        let mut e = app1000(
            1,
            "wmiprvse.exe",
            "perfts.dll",
            "c0000005",
            r"C:\Windows\System32\perfts.dll",
        );
        e.data[10] = r"C:\Windows\system32\wbem\wmiprvse.exe".into();
        let events = vec![
            e.clone(),
            e,
            app1000(1, "TslGame.exe", "unknown", "c0000374", "unknown"),
        ];
        assert!(find(&analyze(&events, &ctx()), "ram_pattern").is_empty());
    }

    #[test]
    fn old_crashes_are_not_marked_as_affecting_now() {
        let events: Vec<SystemEvent> = (40..50)
            .map(|d| app1000(d, "Old.exe", "unknown", "c0000374", "unknown"))
            .collect();
        let r = analyze(&events, &ctx());
        let f = find(&r, "app");
        assert_eq!(
            f[0].impact,
            Impact::Moderate,
            "10 вильотів, але понад місяць тому"
        );
    }

    #[test]
    fn power_losses_depend_on_how_the_user_turns_off_the_pc() {
        let events: Vec<SystemEvent> = (1..=5).map(|d| kp41(d, "0", "0", "0")).collect();
        let r = analyze(&events, &ctx());
        assert_eq!(r.power_losses, 5);
        let f = find(&r, "power");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].impact, Impact::High);
        // Користувач вимикає кнопкою — це інформація, а не тривога.
        let r = analyze(
            &events,
            &CrashContext {
                hard_off_by_button: true,
                now_unix: NOW,
                windows_dir: WIN.into(),
            },
        );
        assert!(find(&r, "power").is_empty());
        assert_eq!(find(&r, "power_user")[0].impact, Impact::Info);
    }

    #[test]
    fn bsod_is_named_and_grouped() {
        let events = vec![
            kp41(1, "278", "0", "0"),
            kp41(3, "278", "0", "0"),
            kp41(5, "292", "0", "0"),
            ev(WHEA_PROVIDER, 18, 5, &[]),
        ];
        let r = analyze(&events, &ctx());
        assert_eq!(r.bsods, 3);
        assert_eq!(r.power_losses, 0);
        let f = find(&r, "bsod");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].title.args["name"], "VIDEO_TDR_FAILURE");
        assert_eq!(f[0].title.args["count"], "2");
        let whea = f[1];
        assert_eq!(whea.title.args["name"], "WHEA_UNCORRECTABLE_ERROR");
        assert!(whea.evidence.iter().any(|e| e.label.key == "crash.ev.whea"));
    }

    #[test]
    fn app_crash_explains_where_and_what() {
        let mut events: Vec<SystemEvent> = (1..=16)
            .map(|d| app1000(d, "TslGame.exe", "unknown", "c0000374", "unknown"))
            .collect();
        events.push(app1000(
            2,
            "Game2.exe",
            "nvwgf2umx.dll",
            "c0000005",
            r"C:\Windows\nvwgf2umx.dll",
        ));
        events.push(ev(DISPLAY_PROVIDER, TDR_EVENT_ID, 2, &["nvlddmkm"]));
        let r = analyze(&events, &ctx());
        assert_eq!(r.app_crashes, 17);
        assert_eq!(r.gpu_resets, 1);
        let apps = find(&r, "app");
        let pubg = apps
            .iter()
            .find(|f| f.title.args["app"] == "TslGame.exe")
            .unwrap();
        assert_eq!(pubg.impact, Impact::High);
        let value = |f: &Finding, label: &str| {
            f.evidence
                .iter()
                .find(|e| e.label.key == label)
                .map(|e| e.value.key.clone())
                .unwrap()
        };
        assert_eq!(value(pubg, "crash.ev.where"), "crash.where.unknown");
        assert_eq!(value(pubg, "crash.ev.what"), "crash.exc.heap");
        assert_eq!(
            value(pubg, "crash.ev.what_to_do"),
            "crash.advice.game_memory"
        );
        let g2 = apps
            .iter()
            .find(|f| f.title.args["app"] == "Game2.exe")
            .unwrap();
        assert_eq!(value(g2, "crash.ev.where"), "crash.where.gpu_nvidia");
        assert_eq!(
            value(g2, "crash.ev.what_to_do"),
            "crash.advice.gpu_driver_tdr"
        );
    }

    #[test]
    fn memory_errors_in_several_games_point_to_ram() {
        let events = vec![
            app1000(1, "TslGame.exe", "unknown", "c0000374", "unknown"),
            app1000(2, "TslGame.exe", "unknown", "c0000374", "unknown"),
            app1000(3, "DayZ_x64.exe", "unknown", "c0000374", "unknown"),
        ];
        let r = analyze(&events, &ctx());
        let f = find(&r, "ram_pattern");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title.args["apps"], "2");
        // Одна програма — ще не закономірність RAM.
        let r = analyze(&events[..2], &ctx());
        assert!(find(&r, "ram_pattern").is_empty());
    }

    /// Кожен текст вердиктів і хронології має переклад і всі аргументи.
    #[test]
    fn every_text_is_localized() {
        let catalog = optimax_core::Catalog::ukrainian().unwrap();
        let mut events: Vec<SystemEvent> = (1..=6).map(|d| kp41(d, "0", "1", "1")).collect();
        for code in ["278", "292", "10", "26", "122", "239", "999999"] {
            events.push(kp41(2, code, "0", "0"));
        }
        events.push(ev(WHEA_PROVIDER, 18, 2, &[]));
        events.push(ev(DISPLAY_PROVIDER, TDR_EVENT_ID, 2, &["nvlddmkm"]));
        let cases = [
            ("unknown", "c0000374", "unknown"),
            ("nvwgf2umx.dll", "c0000005", ""),
            ("amdxx64.dll", "c0000409", ""),
            ("igd12umd64.dll", "c00000fd", ""),
            ("d3d12.dll", "c000001d", ""),
            ("DiscordHook64.dll", "c0000135", ""),
            ("KERNELBASE.dll", "e06d7363", ""),
            ("mod.dll", "e0434352", r"C:\Mods\mod.dll"),
            ("g0.exe", "80000003", ""),
            ("x.dll", "0bad0001", r"C:\Games\g9.exe\x.dll"),
            (
                "Microsoft.UI.Xaml.dll",
                "c0000005",
                r"C:\Windows\SystemApps\x.dll",
            ),
        ];
        for (i, (module, code, path)) in cases.iter().enumerate() {
            events.push(app1000(1, &format!("g{i}.exe"), module, code, path));
        }
        events.push(app1000(1, "g0.exe", "unknown", "c0000374", "unknown"));
        let mut texts = Vec::new();
        for hard_off in [false, true] {
            let r = analyze(
                &events,
                &CrashContext {
                    hard_off_by_button: hard_off,
                    now_unix: NOW,
                    windows_dir: WIN.into(),
                },
            );
            for f in r.findings {
                texts.extend([f.title, f.detail]);
                texts.extend(f.advice);
                for e in f.evidence {
                    texts.extend([e.label, e.value]);
                }
            }
            texts.extend(r.timeline.into_iter().map(|t| t.text));
        }
        let none = analyze(&[], &ctx()).findings.remove(0);
        texts.extend([none.title, none.detail]);
        for t in texts {
            assert!(catalog.contains(&t.key), "немає ключа в uk.json: {}", t.key);
            let r = catalog.render(&t);
            assert!(!r.contains('{'), "незаповнений аргумент у {}: {r}", t.key);
        }
    }

    #[test]
    fn clean_log_is_reported_honestly_and_timeline_is_sorted() {
        let r = analyze(&[], &ctx());
        assert_eq!(find(&r, "none").len(), 1);
        assert!(r.timeline.is_empty());
        let r = analyze(
            &[
                kp41(5, "0", "0", "0"),
                app1000(1, "a.exe", "unknown", "c0000005", "unknown"),
                kp41(3, "278", "0", "0"),
            ],
            &ctx(),
        );
        assert!(r.timeline.windows(2).all(|w| w[0].unix_s >= w[1].unix_s));
        assert_eq!(r.timeline[0].kind, CrashEntryKind::AppCrash);
    }
}

//! Розбір CSV PresentMon за назвами колонок (без урахування регістру).
//!
//! Перевірено на реальному виводі PresentMon 2.6.0 (`TimeInQPC`, `TimeInMs`) і на форматі 1.x
//! (`TimeInSeconds`, `msBetweenPresents`). Значення `NA` означає «немає даних».

use crate::frame::{FrameRecord, FrameTimestamp};
use crate::FrametimeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeColumn {
    Qpc,
    Ms,
    Seconds,
}

/// Парсер рядків CSV з уже розібраним заголовком. Підходить і для потокового читання stdout.
#[derive(Debug, Clone)]
pub struct CsvFrameParser {
    width: usize,
    application: Option<usize>,
    process_id: usize,
    swap_chain: Option<usize>,
    present_mode: Option<usize>,
    time: usize,
    time_kind: TimeColumn,
    between_presents: usize,
    between_display: Option<usize>,
    cpu_busy: Option<usize>,
    gpu_busy: Option<usize>,
    sync_interval: Option<usize>,
    allows_tearing: Option<usize>,
}

/// Ділить рядок CSV на поля з підтримкою лапок і `""` всередині них.
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut cur)),
            '\r' | '\n' if !in_quotes => {}
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

fn find(header: &[String], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .find_map(|n| header.iter().position(|h| h.trim().eq_ignore_ascii_case(n)))
}

fn opt_f64(fields: &[String], idx: Option<usize>) -> Option<f64> {
    let raw = fields.get(idx?)?.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("NA") {
        return None;
    }
    raw.parse::<f64>().ok().filter(|v| v.is_finite())
}

impl CsvFrameParser {
    /// Розбирає рядок заголовка (BOM на початку файлу ігнорується).
    pub fn from_header(line: &str) -> Result<Self, FrametimeError> {
        let header = split_csv_line(line.trim_start_matches('\u{feff}'));
        let (time, time_kind) = if let Some(i) = find(&header, &["TimeInQPC"]) {
            (i, TimeColumn::Qpc)
        } else if let Some(i) = find(&header, &["TimeInMs"]) {
            (i, TimeColumn::Ms)
        } else if let Some(i) = find(&header, &["TimeInSeconds"]) {
            (i, TimeColumn::Seconds)
        } else {
            return Err(FrametimeError::MissingColumn(
                "TimeInQPC/TimeInMs/TimeInSeconds",
            ));
        };
        Ok(Self {
            width: header.len(),
            application: find(&header, &["Application"]),
            process_id: find(&header, &["ProcessID"])
                .ok_or(FrametimeError::MissingColumn("ProcessID"))?,
            swap_chain: find(&header, &["SwapChainAddress"]),
            present_mode: find(&header, &["PresentMode"]),
            time,
            time_kind,
            between_presents: find(&header, &["MsBetweenPresents"])
                .ok_or(FrametimeError::MissingColumn("MsBetweenPresents"))?,
            between_display: find(&header, &["MsBetweenDisplayChange"]),
            cpu_busy: find(&header, &["MsCPUBusy"]),
            gpu_busy: find(&header, &["MsGPUBusy"]),
            sync_interval: find(&header, &["SyncInterval"]),
            allows_tearing: find(&header, &["AllowsTearing"]),
        })
    }

    /// Розбирає рядок даних. `Ok(None)` — порожній рядок або кадр без frametime (перший кадр).
    pub fn parse_line(&self, line: &str) -> Result<Option<FrameRecord>, FrametimeError> {
        if line.trim().is_empty() {
            return Ok(None);
        }
        let f = split_csv_line(line);
        if f.len() < self.width {
            return Err(FrametimeError::ShortLine {
                got: f.len(),
                expected: self.width,
            });
        }
        let Some(frametime) = opt_f64(&f, Some(self.between_presents)).filter(|v| *v > 0.0) else {
            return Ok(None);
        };
        let raw_time = f[self.time].trim();
        let bad_time = || FrametimeError::BadValue {
            column: "TimeIn*",
            value: raw_time.to_string(),
        };
        let time = match self.time_kind {
            TimeColumn::Qpc => FrameTimestamp::Qpc(raw_time.parse().map_err(|_| bad_time())?),
            TimeColumn::Ms => FrameTimestamp::Ms(raw_time.parse().map_err(|_| bad_time())?),
            TimeColumn::Seconds => {
                FrameTimestamp::Ms(raw_time.parse::<f64>().map_err(|_| bad_time())? * 1000.0)
            }
        };
        let raw_pid = f[self.process_id].trim();
        let process_id = raw_pid.parse().map_err(|_| FrametimeError::BadValue {
            column: "ProcessID",
            value: raw_pid.to_string(),
        })?;
        let text = |idx: Option<usize>| {
            idx.and_then(|i| f.get(i))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        Ok(Some(FrameRecord {
            application: text(self.application),
            process_id,
            swap_chain: text(self.swap_chain),
            present_mode: text(self.present_mode),
            time,
            ms_between_presents: frametime,
            ms_between_display_change: opt_f64(&f, self.between_display),
            ms_cpu_busy: opt_f64(&f, self.cpu_busy),
            ms_gpu_busy: opt_f64(&f, self.gpu_busy),
            sync_interval: opt_f64(&f, self.sync_interval)
                .filter(|v| v.fract() == 0.0 && v.abs() <= 16.0)
                .map(|v| v as i32),
            allows_tearing: opt_f64(&f, self.allows_tearing).map(|v| v != 0.0),
        }))
    }
}

/// Розбирає весь CSV. Некоректні рядки пропускаються й рахуються.
pub fn parse_csv(text: &str) -> Result<(Vec<FrameRecord>, usize), FrametimeError> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or(FrametimeError::MissingColumn("заголовок"))?;
    let parser = CsvFrameParser::from_header(header)?;
    let mut frames = Vec::new();
    let mut skipped = 0;
    for line in lines {
        match parser.parse_line(line) {
            Ok(Some(f)) => frames.push(f),
            Ok(None) => {}
            Err(_) => skipped += 1,
        }
    }
    Ok((frames, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::PresentMode;

    #[test]
    fn present_mode_names_round_trip() {
        for (m, name) in PresentMode::NAMES {
            assert_eq!(PresentMode::parse(name), m);
            assert_eq!(m.as_str(), name);
        }
        assert_eq!(
            PresentMode::parse(" composed: flip "),
            PresentMode::ComposedFlip
        );
        assert_eq!(PresentMode::parse("Other"), PresentMode::Other);
        assert_eq!(PresentMode::parse(""), PresentMode::Other);
        assert_eq!(PresentMode::Other.is_composed(), None);
        assert_eq!(PresentMode::ComposedFlip.is_composed(), Some(true));
        assert_eq!(
            PresentMode::HardwareComposedIndependentFlip.is_composed(),
            Some(false)
        );
    }

    /// Справжній вивід PresentMon 2.6.0 з `--qpc_time` (записаний на ПК розробника).
    const REAL_V2_QPC: &str = include_str!("../tests/data/presentmon-2.6.0-qpc.csv");

    #[test]
    fn parses_real_presentmon_2_6_output() {
        let (frames, skipped) = parse_csv(REAL_V2_QPC).unwrap();
        assert_eq!(skipped, 0);
        assert!(frames.len() > 100);
        let f = &frames[0];
        assert_eq!(f.application, "Spotify.exe");
        assert_eq!(f.process_id, 7492);
        assert!(matches!(f.time, FrameTimestamp::Qpc(t) if t > 0));
        assert!(f.ms_between_presents > 0.0);
        assert!(f.ms_gpu_busy.is_some());
        assert_eq!(f.present_mode, "Hardware Composed: Independent Flip");
        assert_eq!(f.sync_interval, Some(0));
        assert_eq!(f.allows_tearing, Some(false));
        // Усі режими з реального файлу розпізнаються.
        for f in &frames {
            assert_ne!(PresentMode::parse(&f.present_mode), PresentMode::Other);
        }
        // У файлі впереміш кадри кількох процесів (не відсортовані глобально),
        // але в межах одного swap chain час зростає.
        assert!(frames.iter().any(|f| f.application != "Spotify.exe"));
        let qpc: Vec<i64> = frames
            .iter()
            .filter(|f| f.application == "Spotify.exe")
            .map(|f| match f.time {
                FrameTimestamp::Qpc(t) => t,
                FrameTimestamp::Ms(_) => 0,
            })
            .collect();
        assert!(qpc.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn parses_v2_ms_header_variant() {
        let csv = "Application,ProcessID,SwapChainAddress,PresentRuntime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,TimeInMs,MsBetweenSimulationStart,MsBetweenPresents,MsBetweenDisplayChange,MsInPresentAPI,MsRenderPresentLatency,MsUntilDisplayed,CPUStartTimeInMs,MsBetweenAppStart,MsCPUBusy,MsCPUWait,MsGPULatency,MsGPUTime,MsGPUBusy,MsGPUWait\n\
            game.exe,10,0x1,DXGI,0,0,1,Hardware: Independent Flip,42.6,NA,6.9,NA,0.05,1.1,4.1,38.1,6.9,5.0,1.9,0.8,6.0,5.5,1.4\n";
        let (frames, _) = parse_csv(csv).unwrap();
        assert_eq!(frames[0].time, FrameTimestamp::Ms(42.6));
        assert_eq!(frames[0].ms_between_display_change, None);
        assert_eq!(frames[0].ms_cpu_busy, Some(5.0));
    }

    #[test]
    fn parses_v1_format() {
        let csv = "Application,ProcessID,SwapChainAddress,Runtime,SyncInterval,PresentFlags,PresentMode,AllowsTearing,TimeInSeconds,msInPresentAPI,msUntilRenderComplete,msUntilDisplayed,Dropped,msBetweenPresents,msBetweenDisplayChange\n\
            game.exe,10,0x1,DXGI,0,0,Hardware: Independent Flip,1,1.5,0.1,1.0,2.0,0,16.6,16.7\n";
        let (frames, _) = parse_csv(csv).unwrap();
        assert_eq!(frames[0].time, FrameTimestamp::Ms(1500.0));
        assert_eq!(frames[0].ms_between_presents, 16.6);
        assert_eq!(frames[0].ms_gpu_busy, None);
    }

    #[test]
    fn handles_quotes_bom_na_and_bad_lines() {
        let csv = "\u{feff}Application,ProcessID,TimeInQPC,MsBetweenPresents\n\
            \"My, \"\"Game\"\".exe\",5,100,NA\n\
            \"My, \"\"Game\"\".exe\",5,200,8.0\n\
            x,notapid,300,8.0\n\
            short\n\
            \n";
        let (frames, skipped) = parse_csv(csv).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].application, "My, \"Game\".exe");
        assert_eq!(skipped, 2);
    }

    #[test]
    fn missing_columns_are_errors() {
        assert!(matches!(
            CsvFrameParser::from_header("Application,ProcessID,MsBetweenPresents"),
            Err(FrametimeError::MissingColumn(_))
        ));
        assert!(matches!(
            CsvFrameParser::from_header("ProcessID,TimeInQPC"),
            Err(FrametimeError::MissingColumn("MsBetweenPresents"))
        ));
    }
}

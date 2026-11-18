//! Визначення процесу гри за текою встановлення.
//!
//! Гра — процес на передньому плані, чий exe лежить у теці гри **і** займає найбільше пам'яті
//! серед процесів цієї теки: так проміжний лаунчер у тій самій теці (`launcher_epic.exe` у
//! Genshin) не сприймається як гра. Вибір має триматися [`STABLE_POLLS`] опитувань поспіль.

use super::norm_dir;

/// Скільки опитувань поспіль має обиратися той самий exe (при опитуванні раз на 2 с — ≈ 6 с).
pub const STABLE_POLLS: u32 = 3;

/// Запущений процес (знімок).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningProcess {
    /// PID.
    pub pid: u32,
    /// Ім'я exe.
    pub name: String,
    /// Повний шлях exe (`None`, якщо Windows не дала).
    pub path: Option<String>,
    /// Робоча пам'ять, МіБ.
    pub memory_mib: u64,
}

/// Чи лежить `path` усередині теки `dir` (без звертань до диска).
pub fn inside_dir(path: &str, dir: &str) -> bool {
    let has_dotdot = |s: &str| s.split(['\\', '/']).any(|c| c.trim() == "..");
    if has_dotdot(path) || has_dotdot(dir) {
        return false;
    }
    let dir = norm_dir(dir);
    if dir.is_empty() {
        return false;
    }
    norm_dir(path)
        .strip_prefix(&dir)
        .is_some_and(|rest| rest.starts_with('\\') && rest.len() > 1)
}

/// Процес гри серед `procs` або `None`, якщо гри ще не видно.
pub fn pick_game<'a>(
    install_dir: &str,
    foreground_pid: Option<u32>,
    procs: &'a [RunningProcess],
) -> Option<&'a RunningProcess> {
    pick_game_in(&[install_dir], foreground_pid, procs, &[])
}

/// Те саме для кількох варіантів теки (шлях із лаунчера й реальний шлях за junction) і з
/// пропуском exe, які вже виявились не грою (`skipped`, без урахування регістру).
pub fn pick_game_in<'a>(
    install_dirs: &[&str],
    foreground_pid: Option<u32>,
    procs: &'a [RunningProcess],
    skipped: &[String],
) -> Option<&'a RunningProcess> {
    let fg = foreground_pid?;
    let in_dir: Vec<&RunningProcess> = procs
        .iter()
        .filter(|p| !skipped.iter().any(|s| s.eq_ignore_ascii_case(&p.name)))
        .filter(|p| {
            p.path
                .as_deref()
                .is_some_and(|path| install_dirs.iter().any(|d| inside_dir(path, d)))
        })
        .collect();
    let heaviest = in_dir.iter().map(|p| p.memory_mib).max()?;
    in_dir
        .into_iter()
        .find(|p| p.pid == fg && p.memory_mib == heaviest)
}

/// Шлях без префікса `\\?\` (так його віддає `std::fs::canonicalize` на Windows).
pub fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

/// Чим закінчився запис гри, процес якої щойно визначено.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingOutcome {
    /// Запис дійшов до кінця (за часом чи гарячою клавішею) і має кадри.
    Finished,
    /// Процес закрився посеред запису — типово для проміжного лаунчера.
    Exited,
    /// Скасовано, не дочекались гри, помилка.
    Other,
}

/// Що зробити з визначеним exe після запису.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRecording {
    /// Запам'ятати назавжди: це справді гра.
    Confirm,
    /// Пропускати до кінця сесії: схоже на лаунчер.
    Skip,
    /// Нічого не вирішувати.
    Nothing,
}

/// Визначений exe зберігається лише після запису, що дійшов до кінця; процес, який закрився
/// посеред запису, пропускається (лаунчер, у якого людина натиснула «Грати»).
pub fn after_recording(outcome: RecordingOutcome) -> AfterRecording {
    match outcome {
        RecordingOutcome::Finished => AfterRecording::Confirm,
        RecordingOutcome::Exited => AfterRecording::Skip,
        RecordingOutcome::Other => AfterRecording::Nothing,
    }
}

/// Стан визначення процесу гри для UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DetectState {
    /// Гри ще не видно на передньому плані.
    Waiting {
        /// Exe, які пропускаються в цій сесії (закрились посеред минулого запису).
        skipped: Vec<String>,
    },
    /// Гру визначено.
    Found {
        /// Ім'я exe.
        exe: String,
    },
    /// У гри немає теки (додана вручну) — визначати нема за чим.
    NoFolder,
}

/// Вимагає, щоб той самий exe обирався [`STABLE_POLLS`] разів поспіль.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detector {
    last: Option<String>,
    streak: u32,
}

impl Detector {
    /// Нове опитування. Повертає ім'я exe, щойно вибір стабільний.
    pub fn feed(&mut self, picked: Option<&str>) -> Option<String> {
        let Some(exe) = picked else {
            *self = Self::default();
            return None;
        };
        let same = self
            .last
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(exe));
        self.streak = if same { self.streak + 1 } else { 1 };
        self.last = Some(exe.to_string());
        (self.streak >= STABLE_POLLS).then(|| exe.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, path: &str, mib: u64) -> RunningProcess {
        let name = path.rsplit('\\').next().unwrap_or(path).to_string();
        RunningProcess {
            pid,
            name,
            path: Some(path.to_string()),
            memory_mib: mib,
        }
    }

    #[test]
    fn inside_dir_nested() {
        assert!(inside_dir(r"F:\Games\Foo\bin\foo.exe", r"F:\Games\Foo"));
    }

    #[test]
    fn inside_dir_rejects_prefix_sibling() {
        assert!(!inside_dir(r"F:\Games\FooBar\x.exe", r"F:\Games\Foo"));
    }

    #[test]
    fn inside_dir_mixed_separators_and_case() {
        assert!(inside_dir(r"f:/games/FOO\Bin/foo.exe", r"F:\Games\Foo"));
    }

    #[test]
    fn inside_dir_trailing_separator() {
        assert!(inside_dir(r"F:\Games\Foo\foo.exe", r"F:\Games\Foo\"));
    }

    #[test]
    fn inside_dir_rejects_dotdot() {
        assert!(!inside_dir(r"F:\Games\Foo\..\Bar\x.exe", r"F:\Games\Foo"));
        assert!(!inside_dir(r"F:\Games\Foo\x.exe", r"F:\Games\Foo\..\Foo"));
    }

    #[test]
    fn inside_dir_empty_dir_is_false() {
        assert!(!inside_dir(r"F:\x.exe", ""));
        assert!(!inside_dir(r"F:\x.exe", "  "));
    }

    #[test]
    fn inside_dir_dir_itself_is_false() {
        assert!(!inside_dir(r"F:\Games\Foo", r"F:\Games\Foo"));
    }

    #[test]
    fn inside_dir_unicode_case() {
        assert!(inside_dir(r"F:\Ігри\Гра\game.exe", r"f:\ігри\гра"));
    }

    #[test]
    fn pick_foreground_heaviest() {
        let procs = [
            p(1, r"F:\EpicGames\RDR2\PlayRDR2.exe", 80),
            p(2, r"F:\EpicGames\RDR2\RDR2.exe", 6000),
            p(3, r"C:\Windows\explorer.exe", 9000),
        ];
        assert_eq!(
            pick_game(r"F:\EpicGames\RDR2", Some(2), &procs).map(|g| g.pid),
            Some(2)
        );
    }

    #[test]
    fn pick_ignores_light_foreground_launcher() {
        let procs = [
            p(1, r"F:\EpicGames\GenshinImpact\launcher_epic.exe", 150),
            p(
                2,
                r"F:\EpicGames\GenshinImpact\Genshin Impact Game\GenshinImpact.exe",
                4000,
            ),
        ];
        assert_eq!(
            pick_game(r"F:\EpicGames\GenshinImpact", Some(1), &procs),
            None
        );
    }

    #[test]
    fn pick_single_process_in_dir() {
        let procs = [p(5, r"F:\Games\KSP\KSP_x64.exe", 3000)];
        assert_eq!(
            pick_game(r"F:\Games\KSP", Some(5), &procs).map(|g| g.pid),
            Some(5)
        );
    }

    #[test]
    fn pick_tie_in_memory_is_ok() {
        let procs = [p(1, r"F:\G\a.exe", 100), p(2, r"F:\G\b.exe", 100)];
        assert_eq!(pick_game(r"F:\G", Some(2), &procs).map(|g| g.pid), Some(2));
    }

    #[test]
    fn pick_none_without_foreground() {
        let procs = [p(5, r"F:\Games\KSP\KSP_x64.exe", 3000)];
        assert_eq!(pick_game(r"F:\Games\KSP", None, &procs), None);
    }

    #[test]
    fn pick_skips_process_without_path() {
        let mut g = p(5, r"F:\Games\KSP\KSP_x64.exe", 3000);
        g.path = None;
        assert_eq!(pick_game(r"F:\Games\KSP", Some(5), &[g]), None);
    }

    #[test]
    fn pick_ignores_other_dirs() {
        let procs = [p(5, r"F:\Games\KSP2\KSP2.exe", 3000)];
        assert_eq!(pick_game(r"F:\Games\KSP", Some(5), &procs), None);
    }

    #[test]
    fn pick_skips_excluded_launcher_alone_in_folder() {
        let procs = [p(1, r"F:\Games\KSP\Launcher.exe", 120)];
        let dirs = [r"F:\Games\KSP"];
        assert_eq!(
            pick_game_in(&dirs, Some(1), &procs, &["launcher.EXE".into()]),
            None
        );
        assert_eq!(
            pick_game_in(&dirs, Some(1), &procs, &[]).map(|g| g.pid),
            Some(1)
        );
    }

    #[test]
    fn pick_excluded_does_not_shadow_game() {
        let procs = [
            p(1, r"F:\Games\KSP\Launcher.exe", 9000),
            p(2, r"F:\Games\KSP\KSP_x64.exe", 3000),
        ];
        let got = pick_game_in(
            &[r"F:\Games\KSP"],
            Some(2),
            &procs,
            &["Launcher.exe".into()],
        );
        assert_eq!(got.map(|g| g.pid), Some(2));
    }

    #[test]
    fn pick_matches_canonical_dir_of_junction() {
        let procs = [p(7, r"E:\Moved\Game\game.exe", 4000)];
        let dirs = [r"D:\Steam\steamapps\common\Game", r"E:\Moved\Game"];
        assert_eq!(
            pick_game_in(&dirs, Some(7), &procs, &[]).map(|g| g.pid),
            Some(7)
        );
    }

    #[test]
    fn strip_verbatim_prefixes() {
        assert_eq!(strip_verbatim(r"\\?\E:\Moved\Game"), r"E:\Moved\Game");
        assert_eq!(
            strip_verbatim(r"\\?\UNC\server\share\g"),
            r"\\server\share\g"
        );
        assert_eq!(strip_verbatim(r"E:\Game"), r"E:\Game");
    }

    #[test]
    fn only_finished_recording_confirms_exe() {
        assert_eq!(
            after_recording(RecordingOutcome::Finished),
            AfterRecording::Confirm
        );
        assert_eq!(
            after_recording(RecordingOutcome::Exited),
            AfterRecording::Skip
        );
        assert_eq!(
            after_recording(RecordingOutcome::Other),
            AfterRecording::Nothing
        );
    }

    #[test]
    fn detector_needs_three_same_polls() {
        let mut d = Detector::default();
        assert_eq!(d.feed(Some("RDR2.exe")), None);
        assert_eq!(d.feed(Some("RDR2.exe")), None);
        assert_eq!(d.feed(Some("RDR2.exe")).as_deref(), Some("RDR2.exe"));
    }

    #[test]
    fn detector_resets_on_change() {
        let mut d = Detector::default();
        d.feed(Some("a.exe"));
        d.feed(Some("a.exe"));
        assert_eq!(d.feed(None), None);
        assert_eq!(d.feed(Some("a.exe")), None);
        d.feed(Some("b.exe"));
        d.feed(Some("b.exe"));
        assert_eq!(d.feed(Some("a.exe")), None);
    }

    #[test]
    fn detector_case_insensitive() {
        let mut d = Detector::default();
        d.feed(Some("RDR2.exe"));
        d.feed(Some("rdr2.EXE"));
        assert!(d.feed(Some("RDR2.exe")).is_some());
    }
}

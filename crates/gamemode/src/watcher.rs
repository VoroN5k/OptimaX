//! Визначення запуску й закриття гри. [`step`] — чиста машина станів, [`scan`] — опитування.

use crate::profile::GameProfile;
use optimax_core::process::{ProcessApi, ProcessInfo};

/// Рекомендований інтервал опитування: достатньо, щоб помітити гру за пару секунд,
/// і майже нульове навантаження (один знімок списку процесів на профіль).
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Стан стежника.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchState {
    /// Жодна гра з профілів не запущена.
    Idle,
    /// Йде сесія гри.
    Active {
        /// Профіль (ім'я exe).
        profile_exe: String,
        /// Процес гри.
        game: ProcessInfo,
    },
}

/// Подія для обробки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Запустилась гра з профілем.
    GameStarted {
        /// Профіль.
        profile_exe: String,
        /// Процес гри.
        game: ProcessInfo,
    },
    /// Гра закрилась.
    GameExited,
}

/// Наступна подія за списком запущених процесів (`(exe профілю, процеси з таким ім'ям)`).
/// Якщо в гри кілька процесів з однаковим ім'ям — береться найстаріший (головний).
pub fn step(state: &WatchState, running: &[(String, Vec<ProcessInfo>)]) -> Option<WatchEvent> {
    match state {
        WatchState::Idle => running.iter().find_map(|(exe, procs)| {
            procs
                .iter()
                .min_by_key(|p| p.start_time)
                .map(|g| WatchEvent::GameStarted {
                    profile_exe: exe.clone(),
                    game: g.clone(),
                })
        }),
        WatchState::Active { profile_exe, game } => {
            let alive = running
                .iter()
                .filter(|(exe, _)| exe.eq_ignore_ascii_case(profile_exe))
                .flat_map(|(_, p)| p)
                .any(|p| p.pid == game.pid && p.start_time == game.start_time);
            (!alive).then_some(WatchEvent::GameExited)
        }
    }
}

/// Опитує процеси для всіх профілів. Помилки окремого профілю ігноруються (логуються).
pub fn scan(
    processes: &dyn ProcessApi,
    profiles: &[GameProfile],
) -> Vec<(String, Vec<ProcessInfo>)> {
    profiles
        .iter()
        .map(|p| {
            let found = processes.find_by_name(&p.exe).unwrap_or_else(|e| {
                tracing::warn!("не вдалося перевірити {}: {e}", p.exe);
                Vec::new()
            });
            (p.exe.clone(), found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use optimax_core::process::MemProcesses;

    #[test]
    fn lifecycle() {
        let procs = MemProcesses::new();
        let profiles = vec![GameProfile::new("game.exe"), GameProfile::new("other.exe")];
        let mut state = WatchState::Idle;
        assert_eq!(step(&state, &scan(&procs, &profiles)), None);

        procs.spawn(50, "game.exe", 20);
        let main = procs.spawn(40, "game.exe", 10);
        let Some(WatchEvent::GameStarted { profile_exe, game }) =
            step(&state, &scan(&procs, &profiles))
        else {
            panic!("очікувався старт гри");
        };
        assert_eq!(game, main, "береться найстаріший процес гри");
        state = WatchState::Active { profile_exe, game };
        assert_eq!(step(&state, &scan(&procs, &profiles)), None);

        // Допоміжний процес закрився — гра ще йде.
        procs.kill(50);
        assert_eq!(step(&state, &scan(&procs, &profiles)), None);
        // Той самий PID, але інший процес (перевикористання PID) — гра закрилась.
        procs.kill(40);
        procs.spawn(40, "game.exe", 99);
        assert_eq!(
            step(&state, &scan(&procs, &profiles)),
            Some(WatchEvent::GameExited)
        );
    }
}

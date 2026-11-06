//! Обмін між вікном програми й процесом навантаження.
//!
//! Процес запускається тим самим exe з [`WORKER_ARG`]. Він пише в stdout рядки JSON
//! ([`WorkerMessage`]), а закриття його stdin означає «зупинись» (тож він зупиниться й тоді, коли
//! програма зникне).

use serde::{Deserialize, Serialize};

/// Аргумент командного рядка, що запускає процес навантаження замість вікна.
pub const WORKER_ARG: &str = "--stability-worker";
/// Найкоротший дозволений тест, с.
pub const MIN_SECONDS: u32 = 5;
/// Найдовший дозволений тест, с (стане більшим разом із нічним режимом).
pub const MAX_SECONDS: u32 = 3600;

/// Повідомлення процесу навантаження.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// Пристрій створено, навантаження починається.
    Started {
        /// Назва відеокарти.
        adapter: String,
        /// Обсяг її пам'яті, МБ.
        vram_mb: u64,
        /// Скільки відеопам'яті перевіряє тест, МБ.
        tested_mb: u64,
    },
    /// Хід тесту (не рідше ніж раз на секунду, поки процес живий).
    Progress {
        /// Минуло, с.
        elapsed_s: f64,
        /// Завдань виконано.
        rounds: u64,
        /// Повторів звірено.
        checked: u64,
        /// Розбіжностей.
        mismatches: u64,
    },
    /// Час вийшов або просили зупинитись.
    Finished {
        /// Минуло, с.
        elapsed_s: f64,
        /// Завдань виконано.
        rounds: u64,
        /// Повторів звірено.
        checked: u64,
        /// Розбіжностей.
        mismatches: u64,
    },
    /// Драйвер скинув відеокарту (TDR) або вона зникла.
    DeviceLost {
        /// HRESULT причини.
        code: u32,
    },
    /// Тест не вдалося виконати (це не висновок про залізо).
    Failed {
        /// Опис для журналу.
        message: String,
    },
}

/// Аргументи процесу навантаження для тесту на `seconds` с.
pub fn worker_args(seconds: u32) -> [String; 2] {
    [WORKER_ARG.to_string(), seconds.to_string()]
}

/// Чи це запуск процесу навантаження, і на скільки секунд. Значення поза межами обрізаються.
pub fn parse_worker_args(args: &[String]) -> Option<u32> {
    if args.first().map(String::as_str) != Some(WORKER_ARG) {
        return None;
    }
    let seconds = args.get(1)?.parse::<u32>().ok()?;
    Some(seconds.clamp(MIN_SECONDS, MAX_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_args_round_trip_and_stay_in_range() {
        assert_eq!(parse_worker_args(&worker_args(600)), Some(600));
        assert_eq!(parse_worker_args(&worker_args(1)), Some(MIN_SECONDS));
        assert_eq!(parse_worker_args(&worker_args(99_999)), Some(MAX_SECONDS));
        assert_eq!(parse_worker_args(&[]), None);
        assert_eq!(parse_worker_args(&["--other".into(), "5".into()]), None);
        assert_eq!(parse_worker_args(&[WORKER_ARG.into()]), None);
        assert_eq!(parse_worker_args(&[WORKER_ARG.into(), "x".into()]), None);
    }

    #[test]
    fn messages_are_one_json_line() {
        let m = WorkerMessage::DeviceLost { code: 0x887A_0006 };
        let line = serde_json::to_string(&m).expect("json");
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<WorkerMessage>(&line).expect("розбір"),
            m
        );
    }
}

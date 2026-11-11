//! Запуск цього самого exe з правами адміністратора (запит UAC).
//!
//! Потрібен, коли програма працює без прав (маніфест `asInvoker`), а служби OptimaX немає: тоді
//! вона перезапускається з правами (один запит UAC при запуску, як було з
//! `requireAdministrator`). Ще — для встановлення й видалення служби з програми без прав.

use crate::CoreError;

#[cfg(windows)]
mod windows;

/// Код Win32: користувач відмовився у вікні UAC.
pub const ERROR_CANCELLED: u32 = 1223;

/// Помилка означає «користувач натиснув «Ні» у вікні UAC».
pub fn is_cancelled(e: &CoreError) -> bool {
    matches!(e, CoreError::Os { code, .. } if *code == ERROR_CANCELLED)
}

/// Рядок параметрів командного рядка: аргумент із пробілом чи лапкою — у лапках (лапки й
/// зворотні скісні перед ними екрануються за правилами `CommandLineToArgvW`).
pub fn command_line(args: &[String]) -> String {
    args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            other => {
                out.extend(std::iter::repeat_n('\\', slashes));
                out.push(other);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// Запускає exe поточного процесу з правами адміністратора й аргументами `args`. `wait` — чекати
/// завершення й повернути код виходу; інакше `None` одразу після запуску. Відмова в UAC —
/// помилка, для якої [`is_cancelled`] дає `true`.
pub fn run_elevated(args: &[String], wait: bool) -> Result<Option<u32>, CoreError> {
    #[cfg(windows)]
    {
        windows::run_elevated(&command_line(args), wait)
    }
    #[cfg(not(windows))]
    {
        let _ = (args, wait);
        Err(CoreError::Unsupported("запуск з правами адміністратора"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_like_command_line_to_argv() {
        let s = |v: &[&str]| command_line(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            s(&["--install-service", "S-1-5-21-1"]),
            "--install-service S-1-5-21-1"
        );
        assert_eq!(s(&[r"C:\Program Files\x"]), r#""C:\Program Files\x""#);
        assert_eq!(s(&[""]), r#""""#);
        assert_eq!(s(&[r#"a"b"#]), r#""a\"b""#);
        // Скісні перед закривальною лапкою подвоюються, щоб не екранувати її.
        assert_eq!(s(&[r"C:\a b\"]), r#""C:\a b\\""#);
        assert_eq!(s(&[r#"x\"y z"#]), r#""x\\\"y z""#);
    }

    #[test]
    fn cancelled_is_recognised() {
        assert!(is_cancelled(&CoreError::Os {
            api: "ShellExecuteExW",
            code: ERROR_CANCELLED
        }));
        assert!(!is_cancelled(&CoreError::Os {
            api: "ShellExecuteExW",
            code: 5
        }));
    }
}

//! Нативні діалоги Windows для ситуацій, коли інтерфейс ще (або вже) недоступний
//! (напр. немає WebView2 — веб-інтерфейс неможливо показати).

/// Питання «Так / Ні» з іконкою попередження. `true` — користувач натиснув «Так».
/// На не-Windows — завжди `false`.
pub fn ask_yes_no(title: &str, text: &str) -> bool {
    imp::ask_yes_no(title, text)
}

/// Повідомлення про помилку з кнопкою «OK».
pub fn show_error(title: &str, text: &str) {
    imp::show_error(title, text);
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO,
    };

    pub(super) fn ask_yes_no(title: &str, text: &str) -> bool {
        // SAFETY: рядки нуль-термінуються HSTRING; вікно-власник не потрібне.
        let r = unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from(title),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        r == IDYES
    }

    pub(super) fn show_error(title: &str, text: &str) {
        // SAFETY: як вище.
        let _ = unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from(title),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn ask_yes_no(_title: &str, _text: &str) -> bool {
        false
    }

    pub(super) fn show_error(title: &str, text: &str) {
        eprintln!("{title}: {text}");
    }
}

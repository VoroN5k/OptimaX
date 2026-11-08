//! Комп'ютер не засинає, поки йде тест: інакше довгий тест зупинить сон, а не залізо.
//!
//! Навантаження саме по собі сон не відкладає: Windows рахує час без введення користувача.
//! Екран при цьому гасне як завжди — це не заважає тесту.

use std::marker::PhantomData;

/// Поки значення живе, система не засинає. Діє для потоку, що його створив (так влаштований
/// `SetThreadExecutionState`), тому між потоками не передається.
#[derive(Debug)]
pub struct KeepAwake {
    _same_thread: PhantomData<*const ()>,
}

impl KeepAwake {
    /// Забороняє сон для поточного потоку. `None` — Windows відмовила (або це не Windows).
    pub fn start() -> Option<Self> {
        imp::set(true).then_some(Self {
            _same_thread: PhantomData,
        })
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        imp::set(false);
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };

    pub fn set(awake: bool) -> bool {
        let flags = if awake {
            ES_CONTINUOUS | ES_SYSTEM_REQUIRED
        } else {
            ES_CONTINUOUS
        };
        // SAFETY: функція лише змінює стан живлення поточного потоку, вказівників не приймає.
        unsafe { SetThreadExecutionState(flags) }.0 != 0
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn set(_awake: bool) -> bool {
        false
    }
}

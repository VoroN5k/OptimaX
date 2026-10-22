//! Точка відновлення Windows перед першою зміною в сесії.

use optimax_core::CoreError;
use serde::{Deserialize, Serialize};

/// Що сталося з точкою відновлення.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RestorePointStatus {
    /// Windows повідомила про створення точки з цим номером.
    Created {
        /// Номер послідовності System Restore.
        sequence: i64,
    },
    /// Створити не вдалося, користувач погодився продовжити без неї.
    SkippedByUser {
        /// Причина невдачі.
        reason: String,
    },
}

/// Створення точки відновлення.
pub trait RestorePointApi: Send + Sync {
    /// Створює точку й повертає її номер.
    fn create(&self, description: &str) -> Result<i64, CoreError>;
}

/// Мок для тестів: завжди повертає задане.
#[derive(Debug, Clone)]
pub struct FakeRestore(pub Result<i64, u32>);

impl RestorePointApi for FakeRestore {
    fn create(&self, _description: &str) -> Result<i64, CoreError> {
        self.0.map_err(|code| CoreError::Os {
            api: "SRSetRestorePointW",
            code,
        })
    }
}

#[cfg(windows)]
pub use self::windows::WinRestorePoint;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use super::RestorePointApi;
    use optimax_core::CoreError;
    use windows::Win32::System::Restore::{
        SRSetRestorePointW, BEGIN_SYSTEM_CHANGE, END_SYSTEM_CHANGE, MODIFY_SETTINGS,
        RESTOREPOINTINFOW, STATEMGRSTATUS,
    };

    /// System Restore. Потребує прав адміністратора й увімкненого захисту системи.
    ///
    /// Увага: Windows типово створює не більше однієї точки на 24 години
    /// (`SystemRestorePointCreationFrequency`); що саме тоді повертає API — треба перевірити на ПК.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WinRestorePoint;

    fn call(info: &RESTOREPOINTINFOW) -> Result<i64, CoreError> {
        let mut status = STATEMGRSTATUS::default();
        // SAFETY: обидві структури валідні на час виклику, опис нуль-термінований.
        let ok = unsafe { SRSetRestorePointW(info, &mut status) }.as_bool();
        if ok {
            Ok(status.llSequenceNumber)
        } else {
            Err(CoreError::Os {
                api: "SRSetRestorePointW",
                code: status.nStatus.0,
            })
        }
    }

    impl RestorePointApi for WinRestorePoint {
        fn create(&self, description: &str) -> Result<i64, CoreError> {
            // Опис обрізаємо, лишаючи місце під нуль у кінці. Структура упакована (packed),
            // тому масив збираємо окремо й присвоюємо цілим.
            let mut desc = [0u16; 256];
            for (dst, src) in desc.iter_mut().zip(description.encode_utf16().take(255)) {
                *dst = src;
            }
            let begin = RESTOREPOINTINFOW {
                dwEventType: BEGIN_SYSTEM_CHANGE,
                dwRestorePtType: MODIFY_SETTINGS,
                szDescription: desc,
                ..Default::default()
            };
            let sequence = call(&begin)?;
            let end = RESTOREPOINTINFOW {
                dwEventType: END_SYSTEM_CHANGE,
                dwRestorePtType: MODIFY_SETTINGS,
                llSequenceNumber: sequence,
                ..Default::default()
            };
            call(&end)?;
            Ok(sequence)
        }
    }
}

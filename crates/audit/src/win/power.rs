//! Активна схема живлення (через спільний [`WinPower`] з `optimax-core`).

use crate::model::PowerPlan;
use optimax_core::power::{PowerApi, WinPower};
use optimax_core::CoreError;

pub(super) fn read() -> Result<PowerPlan, CoreError> {
    let power = WinPower;
    let guid = power.active_scheme()?;
    Ok(PowerPlan {
        name: power.scheme_name(&guid),
        guid,
    })
}

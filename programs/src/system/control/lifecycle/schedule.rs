use super::{self as work, Key, dispatch};
use crate::system::control::{Fail as ControlFail, core::verdict::Fail};
use ::schedule::{BuildError, Plan, Schedule};
use alloc::vec::Vec;
pub fn lifecycle() -> Result<Vec<(Key, Plan<Fail>)>, BuildError> {
    let mut mint = Schedule::sequence();
    mint.system("validate", work::mint::pre)?;
    mint.system("create", work::mint::run)?;
    mint.system("commit", work::mint::post)?;
    let mut embark = Schedule::sequence();
    embark.system("bind", work::embark::pre)?;
    embark.system("activation", work::embark::activation)?;
    embark.system("run", work::embark::run)?;
    embark.system("supply", work::embark::supply)?;
    embark.system("ready", work::embark::ready)?;
    embark.system("commit", work::embark::post)?;
    let mut debark = Schedule::sequence();
    debark.system("validate", work::debark::pre)?;
    debark.system("pause", work::debark::run)?;
    debark.system("commit", work::debark::post)?;
    let mut ruin = Schedule::sequence();
    ruin.system("activation", work::ruin::activation)?;
    ruin.system("retire", work::ruin::pre)?;
    ruin.system("destroy", work::ruin::run)?;
    ruin.system("reclaim", work::ruin::post)?;
    Ok(alloc::vec![
        (Key::Mint, mint.build()?),
        (Key::Embark, embark.build()?),
        (Key::Debark, debark.build()?),
        (Key::Ruin, ruin.build()?)
    ])
}
pub fn actions(children: Vec<(Key, Plan<Fail>)>) -> Result<Plan<ControlFail>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.system("budget", dispatch::budget)?;
    plan.subplans("lifecycle", dispatch::select, children, dispatch::finish)?;
    plan.system("failed.mint", work::mint::failed)?;
    plan.build()
}

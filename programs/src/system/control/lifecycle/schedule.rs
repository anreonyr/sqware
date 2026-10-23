use super::{self as work, Key, dispatch};
use crate::system::app::Fault as ControlFail;
use crate::system::control::unit::verdict::Fail;

use ::schedule::{BuildError, Plan, Schedule};
use alloc::vec::Vec;
pub(crate) struct ActivationHooks {
    pub construct: Plan<Fail>,
    pub prepare: Plan<Fail>,
    pub supply: Plan<Fail>,
    pub retire: Plan<Fail>,
}
pub fn lifecycle(hooks: ActivationHooks) -> Result<Vec<(Key, Plan<Fail>)>, BuildError> {
    let mut mint = Schedule::sequence();
    mint.system("admit", work::authority::admit)?;
    mint.system("validate", work::mint::pre)?;
    mint.plan("create", hooks.construct)?;
    mint.system("commit", work::mint::post)?;
    let mut embark = Schedule::sequence();
    embark.system("authorize", work::authority::recheck)?;
    embark.system("bind", work::embark::pre)?;
    embark.plan("activation", hooks.prepare)?;
    embark.system("run", work::embark::run)?;
    embark.plan("supply", hooks.supply)?;
    embark.system("ready", work::embark::ready)?;
    embark.system("commit", work::embark::post)?;
    let mut debark = Schedule::sequence();
    debark.system("validate", work::debark::pre)?;
    debark.system("pause", work::debark::run)?;
    debark.system("commit", work::debark::post)?;
    let mut ruin = Schedule::sequence();
    ruin.system("drain", work::ruin::drain)?;
    ruin.system("retire", work::ruin::pre)?;
    ruin.system("guard", work::ruin::release_guard)?;
    ruin.plan("activation", hooks.retire)?;
    ruin.system("destroy", work::ruin::run)?;
    ruin.system("construction", work::ruin::retire_construction)?;
    ruin.system("namespace", work::ruin::post)?;
    ruin.system("supply", work::ruin::release_supply)?;
    ruin.system("identity", work::ruin::retire_identity)?;
    ruin.system("reclaim", work::ruin::finalize)?;
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

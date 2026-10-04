use crate::system::control::{
    core::verdict::Fail,
    serve::{
        self,
        lifecycle::{self as work, Key},
    },
};
use alloc::vec::Vec;
use protocol::common::schedule::{BuildError, Phase, Plan, Schedule};
pub fn lifecycle() -> Result<Vec<(Key, Plan<Fail>)>, BuildError> {
    let mut mint = Schedule::new();
    mint.add_system("validate", Phase::PreMint, work::mint::pre)?;
    mint.add_system("create", Phase::Mint, work::mint::run)?;
    mint.add_system("commit", Phase::PostMint, work::mint::post)?;
    let mut embark = Schedule::new();
    embark.add_system("bind", Phase::PreEmbark, work::embark::pre)?;
    embark.add_system("activation", Phase::PreEmbark, work::embark::activation)?;
    embark.add_system("run", Phase::Embark, work::embark::run)?;
    embark.add_system("supply", Phase::Embark, work::embark::supply)?;
    embark.add_system("ready", Phase::Embark, work::embark::ready)?;
    embark.add_system("commit", Phase::PostEmbark, work::embark::post)?;
    embark.before("bind", "activation")?;
    embark.before("run", "supply")?;
    embark.before("supply", "ready")?;
    let mut debark = Schedule::new();
    debark.add_system("validate", Phase::PreDebark, work::debark::pre)?;
    debark.add_system("pause", Phase::Debark, work::debark::run)?;
    debark.add_system("commit", Phase::PostDebark, work::debark::post)?;
    let mut ruin = Schedule::new();
    ruin.add_system("activation", Phase::PreRuin, work::ruin::activation)?;
    ruin.add_system("retire", Phase::PreRuin, work::ruin::pre)?;
    ruin.add_system("destroy", Phase::Ruin, work::ruin::run)?;
    ruin.add_system("reclaim", Phase::PostRuin, work::ruin::post)?;
    ruin.before("activation", "retire")?;
    Ok(alloc::vec![
        (Key::Mint, mint.build()?),
        (Key::Embark, embark.build()?),
        (Key::Debark, debark.build()?),
        (Key::Ruin, ruin.build()?)
    ])
}
pub fn actions(children: Vec<(Key, Plan<Fail>)>) -> Result<Plan<serve::Fail>, BuildError> {
    let mut plan = Schedule::new();
    plan.add_system("budget", 0u8, serve::driver::budget)?;
    plan.add_subplans(
        "lifecycle",
        1,
        serve::driver::select,
        children,
        serve::driver::finish,
    )?;
    plan.add_system("failed.mint", 2, serve::driver::failed_mint)?;
    plan.build()
}

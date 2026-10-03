use protocol::common::schedule::{Phase, Schedule, Plan, BuildError};
use crate::system::control::{core::verdict::Fail, serve::lifecycle as work};

pub struct Lifecycle {
    pub mint: Plan<Fail>, pub embark: Plan<Fail>,
    pub debark: Plan<Fail>, pub ruin: Plan<Fail>,
}
pub fn lifecycle() -> Result<Lifecycle, BuildError> {
    let mut mint = Schedule::new();
    mint.add_system("validate", Phase::PreMint, work::mint::pre)?;
    mint.add_system("create", Phase::Mint, work::mint::run)?;
    mint.add_system("commit", Phase::PostMint, work::mint::post)?;
    let mut embark = Schedule::new();
    embark.add_system("bind", Phase::PreEmbark, work::embark::pre)?;
    embark.add_system("run", Phase::Embark, work::embark::run)?;
    embark.add_system("ready", Phase::PostEmbark, work::embark::post)?;
    let mut debark = Schedule::new();
    debark.add_system("validate", Phase::PreDebark, work::debark::pre)?;
    debark.add_system("pause", Phase::Debark, work::debark::run)?;
    debark.add_system("commit", Phase::PostDebark, work::debark::post)?;
    let mut ruin = Schedule::new();
    ruin.add_system("retire", Phase::PreRuin, work::ruin::pre)?;
    ruin.add_system("destroy", Phase::Ruin, work::ruin::run)?;
    ruin.add_system("reclaim", Phase::PostRuin, work::ruin::post)?;
    Ok(Lifecycle { mint: mint.build()?, embark: embark.build()?, debark: debark.build()?, ruin: ruin.build()? })
}

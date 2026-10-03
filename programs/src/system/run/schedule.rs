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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CyclePhase { Connect, Snapshot, Retire, Prepare, Receive }

pub fn cycle() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::{control::serve::{living, publication, resource},
        identity::serve::names, operator::serve::install};
    let mut schedule = Schedule::new();
    schedule.add_system("activation", CyclePhase::Connect, crate::service::hub::bridge::maintain)?;
    schedule.add_system("connect", CyclePhase::Connect, install::connect)?;
    schedule.before("activation", "connect")?;
    schedule.add_system("living", CyclePhase::Snapshot, living::capture)?;
    schedule.add_system("publications.retire", CyclePhase::Retire, publication::retire)?;
    schedule.add_system("names.retire", CyclePhase::Retire, names::retire)?;
    schedule.add_system("resources.retire", CyclePhase::Retire, resource::retire)?;
    schedule.before("publications.retire", "names.retire")?;
    schedule.before("names.retire", "resources.retire")?;
    schedule.add_system("resources.prepare", CyclePhase::Prepare, resource::prepare)?;
    schedule.add_system("names.prepare", CyclePhase::Prepare, names::prepare)?;
    schedule.before("resources.prepare", "names.prepare")?;
    schedule.add_system("publications.receive", CyclePhase::Receive, publication::receive)?;
    schedule.add_system("names.receive", CyclePhase::Receive, names::receive)?;
    schedule.before("publications.receive", "names.receive")?;
    schedule.build()
}

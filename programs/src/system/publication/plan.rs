use ::schedule::{BuildError, Plan, Schedule};

pub fn maintenance() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::{
        publication::living,
        publication::names,
        publication::{self as p, runtime as resource},
    };
    let mut request = Schedule::sequence();
    request.system("reset", p::install::reset)?;
    request.system("simple", p::install::simple)?;
    request.system("unpublish", p::install::unpublish)?;
    request.system("withdraw", p::install::withdraw)?;
    request.system("source", p::policy::source)?;
    request.system("service", p::policy::service)?;
    request.system("alias.permission", p::policy::alias)?;
    request.system("runtime", p::policy::runtime)?;
    request.system("identity", p::policy::identity)?;
    request.system("existing", p::install::existing)?;
    request.system("kind.prepare", p::install::prepare_kind)?;
    request.system("install", p::install::install)?;
    request.system("kind.commit", p::install::commit_kind)?;
    request.system("alias", p::install::alias)?;
    request.system("commit", p::install::commit)?;
    request.system("reply", p::receive::finish)?;
    let mut schedule = Schedule::sequence();
    schedule.system("connect", crate::system::operator::connect)?;
    schedule.system("living", living::capture)?;
    schedule.system("authority", living::authority)?;
    schedule.system("operator", living::operator)?;
    schedule.system("publications.retire", p::retire::retire)?;
    schedule.system("names.expired", names::expired)?;
    schedule.system("names.retire", names::retire)?;
    schedule.system("resources.retire", resource::retire)?;
    schedule.system("resources.candidates", resource::candidates)?;
    schedule.system("resources.prepare", resource::prepare)?;
    schedule.system("resources.install", resource::install)?;
    schedule.system("names.changes", names::changes)?;
    schedule.system("names.prepare", names::prepare)?;
    schedule.system("names.select", names::select)?;
    schedule.system("names.verify", names::verify)?;
    schedule.system("names.install", names::install)?;
    schedule.system("publications.receive", p::receive::receive)?;
    schedule.subplans(
        "publications",
        p::receive::select,
        alloc::vec![(0u8, request.build()?)],
        p::receive::completed,
    )?;
    schedule.system("names.receive", names::receive)?;
    schedule.build()
}

pub(crate) fn prepare_runtime() -> Result<Plan<&'static str>, BuildError> {
    let mut prepare = Schedule::sequence();
    prepare.system("runtime.candidates", super::runtime::candidates)?;
    prepare.system("runtime.prepare", super::runtime::prepare)?;
    prepare.system("runtime.install", super::runtime::install)?;
    prepare.build()
}
pub(crate) fn retire_tasks() -> Result<Plan<&'static str>, BuildError> {
    use super::{living, names};
    let mut retire = Schedule::sequence();
    retire.system("living", living::capture)?;
    retire.system("authority", living::authority)?;
    retire.system("operator", living::operator)?;
    retire.system("publications", crate::system::publication::retire::retire)?;
    retire.system("names.expired", names::expired)?;
    retire.system("names.retire", names::retire)?;
    retire.system("runtime", crate::system::publication::runtime::retire)?;
    retire.build()
}

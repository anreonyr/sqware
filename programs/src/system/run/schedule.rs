use crate::system::control::{lifecycle::schedule as lifecycle, serve};
use ::schedule::{BuildError, Plan, Schedule};
pub fn maintenance() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::{
        run::connections,
        run::living,
        run::names,
        run::{publication as p, resource},
    };
    let mut request = Schedule::sequence();
    request.system("reset", p::install::reset)?;
    request.system("simple", p::install::simple)?;
    request.system("unpublish", p::install::unpublish)?;
    request.system("withdraw", p::install::withdraw)?;
    request.system("source", p::policy::source)?;
    request.system("service", p::policy::service)?;
    request.system("device", p::policy::device)?;
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
    schedule.system("activation", crate::service::hub::bridge::maintain)?;
    schedule.system("candidates", connections::candidates)?;
    schedule.system("connect", connections::connect)?;
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
pub fn startup() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::boot;
    let mut start = Schedule::sequence();
    start.system("spawn", boot::spawn)?;
    start.system("embark", boot::embark)?;
    start.system("adopt", boot::adopt)?;
    start.system("identity", boot::identity)?;
    start.system("wire", boot::wire)?;
    start.system(
        "identity.faces",
        crate::system::run::publication::identity::faces,
    )?;
    start.system("identity.publish", boot::publish)?;
    start.system("control.name", boot::name)?;
    start.system(
        "operator.faces",
        crate::system::run::publication::operator::faces,
    )?;
    start.system(
        "publication.face",
        super::publication::faces::publication_face,
    )?;
    start.system("account.initialize", super::account::initialize)?;
    start.system("account.identity", super::account::account)?;
    start.system("account.publication", super::account::publication)?;
    start.system("control.faces", super::publication::faces::faces)?;
    start.system("instance.face", crate::system::run::instances::publication)?;
    start.system("loader", crate::system::run::loading::publication::faces)?;
    start.system("publish", boot::publish)?;
    let mut units = Schedule::sequence();
    units.system("begin", crate::system::run::frame::startup)?;
    units.system("running", crate::system::run::frame::running)?;
    start.plan(
        "static",
        units.build()?.map_error(|_| "static unit startup"),
    )?;
    start.system("running", boot::await_running)?;
    start.build()
}
pub fn frame() -> Result<Plan<serve::Fail>, BuildError> {
    use super::frame as f;
    use super::watch;
    use serve::answer;
    let mut frame = Schedule::sequence();
    frame.plan(
        "maintain.before",
        maintenance()?.map_error(|_| serve::Fail::Publication),
    )?;
    frame.system("health", f::health)?;
    frame.system("reap", serve::reap::sweep)?;
    frame.system("instances.receive", serve::instance::receive)?;
    frame.system("requests.receive", answer::receive)?;
    frame.system("requests.state", answer::state)?;
    frame.system("accounts.receive", super::account::receive)?;
    frame.plan("loader", crate::system::run::loading::schedule::frame()?)?;
    frame.system("requests.instances", serve::instance::answer)?;
    frame.system("requests.enqueue", answer::enqueue)?;
    frame.plan("lifecycle", lifecycle::actions(lifecycle::lifecycle()?)?)?;
    frame.plan(
        "maintain.after",
        maintenance()?.map_error(|_| serve::Fail::Publication),
    )?;
    frame.system("instances.reap", crate::system::run::instances::reap)?;
    frame.system("replies", f::reply)?;
    frame.plan("instance.hooks", super::hooks::instance()?)?;
    frame.system("static", f::startup)?;
    frame.system("retire.static", f::retire_static)?;
    frame.system("launch.completed", super::launch::completed)?;
    frame.system("running", f::running)?;
    frame.system("activity", f::activity)?;
    frame.system("eligibility", f::eligibility)?;
    frame.system("settle", f::settle)?;
    frame.system("idle", f::idle)?;
    frame.system("deadline", f::deadline)?;
    frame.system("ruin.rest", f::ruin_rest)?;
    frame.system("done", f::done)?;
    frame.system("bound", f::bound)?;
    frame.system("stopping.bound", f::stopping_bound)?;
    frame.system("instances.pending", crate::system::run::instances::pending)?;
    frame.system("pending", f::pending)?;
    frame.system("watch.entries", watch::entries)?;
    frame.system("watch.account", super::account::watch)?;
    frame.system("watch.identity", watch::identity_changes)?;
    frame.system("watch.loader", crate::system::run::loading::watch::entries)?;
    frame.system("watch.publication", watch::publication)?;
    frame.system("watch.activation", watch::activation)?;
    frame.system("watch.tasks", watch::tasks)?;
    frame.system("watch.apply", watch::apply)?;
    frame.system("wait", watch::wait)?;
    frame.build()
}
pub fn shutdown() -> Result<Plan<serve::Fail>, BuildError> {
    let mut stop = Schedule::sequence();
    stop.plan(
        "loader.close",
        crate::system::run::loading::schedule::shutdown()?,
    )?;
    stop.system("stopping", crate::system::life::stopping)?;
    stop.system("join", crate::system::life::join)?;
    stop.build()
}

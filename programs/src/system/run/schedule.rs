use crate::system::control::serve;
use protocol::common::schedule::{BuildError, Plan, Schedule};
pub fn maintenance() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::{
        control::serve::living,
        identity::serve::names,
        operator::serve::install,
        run::{publication as p, resource},
    };
    let mut request = Schedule::new();
    request.add_system("reset", 0u8, p::install::reset)?;
    request.add_system("simple", 1, p::install::simple)?;
    request.add_system("unpublish", 2, p::install::unpublish)?;
    request.add_system("withdraw", 2, p::install::withdraw)?;
    request.before("unpublish", "withdraw")?;
    request.add_system("source", 3, p::policy::source)?;
    request.add_system("service", 4, p::policy::service)?;
    request.add_system("device", 5, p::policy::device)?;
    request.add_system("runtime", 6, p::policy::runtime)?;
    request.add_system("identity", 7, p::policy::identity)?;
    request.add_system("existing", 8, p::install::existing)?;
    request.add_system("kind.prepare", 9, p::install::prepare_kind)?;
    request.add_system("install", 10, p::install::install)?;
    request.add_system("kind.commit", 11, p::install::commit_kind)?;
    request.add_system("alias", 12, p::install::alias)?;
    request.add_system("commit", 13, p::install::commit)?;
    request.add_system("reply", 14, p::receive::finish)?;
    let mut schedule = Schedule::new();
    schedule.add_system("activation", 0u8, crate::service::hub::bridge::maintain)?;
    schedule.add_system("candidates", 1, install::candidates)?;
    schedule.add_system("connect", 1, install::connect)?;
    schedule.before("candidates", "connect")?;
    schedule.add_system("living", 2, living::capture)?;
    schedule.add_system("authority", 3, living::authority)?;
    schedule.add_system("operator", 4, living::operator)?;
    schedule.add_system("publications.retire", 5, p::retire::retire)?;
    schedule.add_system("names.expired", 6, names::expired)?;
    schedule.add_system("names.retire", 7, names::retire)?;
    schedule.add_system("resources.retire", 8, resource::retire)?;
    schedule.add_system("resources.candidates", 9, resource::candidates)?;
    schedule.add_system("resources.prepare", 9, resource::prepare)?;
    schedule.before("resources.candidates", "resources.prepare")?;
    schedule.add_system("resources.install", 10, resource::install)?;
    schedule.add_system("names.changes", 11, names::changes)?;
    schedule.add_system("names.prepare", 11, names::prepare)?;
    schedule.before("names.changes", "names.prepare")?;
    schedule.add_system("names.select", 12, names::select)?;
    schedule.add_system("names.verify", 12, names::verify)?;
    schedule.add_system("names.install", 12, names::install)?;
    schedule.before("names.select", "names.verify")?;
    schedule.before("names.verify", "names.install")?;
    schedule.add_system("publications.receive", 13, p::receive::receive)?;
    schedule.add_subplans(
        "publications",
        14,
        p::receive::select,
        alloc::vec![(0u8, request.build()?)],
        p::receive::completed,
    )?;
    schedule.add_system("names.receive", 15, names::receive)?;
    schedule.build()
}
pub fn startup() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::boot;
    let mut start = Schedule::new();
    start.add_system("spawn", 0u8, boot::spawn)?;
    start.add_system("embark", 1, boot::embark)?;
    start.add_system("adopt", 2, boot::adopt)?;
    start.add_system("identity", 3, boot::identity)?;
    start.add_system("wire", 4, boot::wire)?;
    start.add_system(
        "identity.faces",
        5,
        crate::system::identity::serve::publication::faces,
    )?;
    start.add_system("identity.publish", 6, boot::publish)?;
    start.add_system("control.name", 7, boot::name)?;
    start.add_system(
        "operator.faces",
        8,
        crate::system::operator::serve::publication::faces,
    )?;
    start.add_system(
        "publication.face",
        9,
        super::publication::faces::publication_face,
    )?;
    start.add_system("control.faces", 10, super::publication::faces::faces)?;
    start.add_system("instance.face", 10, serve::instance::publication)?;
    start.add_system(
        "loader",
        10,
        crate::system::run::loading::publication::faces,
    )?;
    start.add_system("account.initialize", 10, super::account::initialize)?;
    start.add_system("account.identity", 10, super::account::account)?;
    start.before("account.initialize", "account.identity")?;
    start.add_system("account.publication", 10, super::account::publication)?;
    start.add_system("publish", 11, boot::publish)?;
    let mut units = Schedule::new();
    units.add_system("begin", 0u8, serve::frame::startup)?;
    units.add_system("running", 1, serve::frame::running)?;
    start.add_plan(
        "static",
        12,
        units.build()?.map_error(|_| "static unit startup"),
    )?;
    start.add_system("running", 13, boot::await_running)?;
    start.build()
}
pub fn frame() -> Result<Plan<serve::Fail>, BuildError> {
    use serve::{answer, frame as f, watch};
    let mut frame = Schedule::new();
    frame.add_plan(
        "maintain.before",
        0u8,
        maintenance()?.map_error(|_| serve::Fail::Publication),
    )?;
    frame.add_system("health", 1, f::health)?;
    frame.add_system("reap", 2, serve::reap::sweep)?;
    frame.add_system("requests.receive", 3, answer::receive)?;
    frame.add_system("requests.state", 4, answer::state)?;
    frame.add_system("instances.receive", 3, serve::instance::receive)?;
    frame.add_system("requests.instances", 5, serve::instance::answer)?;
    frame.add_system("accounts.receive", 5, super::account::receive)?;
    frame.add_system("requests.enqueue", 5, answer::enqueue)?;
    frame.before("requests.instances", "requests.enqueue")?;
    frame.add_plan("loader", 5, crate::system::run::loading::schedule::frame()?)?;
    frame.before("loader", "requests.instances")?;
    frame.add_system("instances.reap", 8, serve::instance::reap)?;
    frame.add_plan(
        "lifecycle",
        6,
        serve::schedule::actions(serve::schedule::lifecycle()?)?,
    )?;
    frame.add_plan(
        "maintain.after",
        7,
        maintenance()?.map_error(|_| serve::Fail::Publication),
    )?;
    frame.add_plan("instance.hooks", 9, super::hooks::instance()?)?;
    frame.add_system("launch.completed", 10, super::launch::completed)?;
    frame.before("instance.hooks", "launch.completed")?;
    frame.add_system("replies", 8, f::reply)?;
    frame.add_system("static", 9, f::startup)?;
    frame.add_system("retire.static", 9, f::retire_static)?;
    frame.before("static", "retire.static")?;
    frame.add_system("running", 10, f::running)?;
    frame.add_system("activity", 11, f::activity)?;
    frame.add_system("eligibility", 12, f::eligibility)?;
    frame.add_system("settle", 13, f::settle)?;
    frame.add_system("idle", 14, f::idle)?;
    frame.add_system("deadline", 15, f::deadline)?;
    frame.add_system("ruin.rest", 16, f::ruin_rest)?;
    frame.add_system("done", 17, f::done)?;
    frame.add_system("bound", 18, f::bound)?;
    frame.add_system("stopping.bound", 18, f::stopping_bound)?;
    frame.before("bound", "stopping.bound")?;
    frame.add_system("pending", 19, f::pending)?;
    frame.add_system("instances.pending", 19, serve::instance::pending)?;
    frame.add_system("watch.entries", 20, watch::entries)?;
    frame.add_system(
        "watch.loader",
        21,
        crate::system::run::loading::watch::entries,
    )?;
    frame.add_system("watch.publication", 21, watch::publication)?;
    frame.add_system("watch.account", 21, super::account::watch)?;
    frame.add_system("watch.identity", 21, watch::identity_changes)?;
    frame.add_system("watch.activation", 22, watch::activation)?;
    frame.add_system("watch.tasks", 23, watch::tasks)?;
    frame.add_system("watch.apply", 24, watch::apply)?;
    frame.add_system("wait", 25, watch::wait)?;
    frame.build()
}
pub fn shutdown() -> Result<Plan<serve::Fail>, BuildError> {
    let mut stop = Schedule::new();
    stop.add_plan(
        "loader.close",
        0u8,
        crate::system::run::loading::schedule::shutdown()?,
    )?;
    stop.add_system("stopping", 0u8, crate::system::life::stopping)?;
    stop.before("loader.close", "stopping")?;
    stop.add_system("join", 1, crate::system::life::join)?;
    stop.build()
}

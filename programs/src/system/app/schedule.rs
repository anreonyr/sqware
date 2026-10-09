use crate::system::app::Fault as Fail;

use ::schedule::{BuildError, Plan, Schedule};
pub fn maintenance() -> Result<Plan<&'static str>, BuildError> {
    let mut plan = Schedule::sequence();
    plan.plan("publication", crate::system::publication::maintenance()?)?;
    plan.build()
}
pub fn startup() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::app::boot;
    let mut start = Schedule::sequence();
    start.system("spawn", boot::spawn)?;
    start.system("embark", boot::embark)?;
    start.system("adopt", boot::adopt)?;
    start.system("identity", boot::identity)?;
    start.system("wire", boot::wire)?;
    start.system("identity.faces", crate::system::publication::identity_faces)?;
    start.system("identity.publish", boot::publish)?;
    start.system("control.name", boot::name)?;
    start.system(
        "publication.face",
        crate::system::publication::publication_face,
    )?;
    start.system("control.faces", crate::system::publication::control_faces)?;
    start.system("instance.face", crate::system::publication::instance_face)?;
    start.system("loader", crate::system::loader::faces)?;
    start.system("publish", boot::publish)?;
    let mut units = Schedule::sequence();
    units.system("begin", crate::system::control::startup)?;
    units.system("running", crate::system::app::policy::running)?;
    start.plan(
        "static",
        units.build()?.map_error(|_| "static unit startup"),
    )?;
    start.system("running", boot::await_running)?;
    start.build()
}
pub fn frame() -> Result<Plan<Fail>, BuildError> {
    use super::policy as f;
    use super::wait as watch;
    let mut frame = Schedule::sequence();
    frame.plan(
        "maintain.before",
        maintenance()?.map_error(|_| Fail::Publication),
    )?;
    frame.system("health", f::health)?;
    frame.plan("control.poll", crate::system::control::poll()?)?;
    frame.system(
        "construction.receive",
        crate::system::control::receive_construction,
    )?;
    frame.system(
        "construction.admit",
        crate::system::control::admit_construction,
    )?;
    frame.system("construction.dispatch", crate::system::launch::dispatch)?;
    frame.plan("loader", crate::system::loader::frame()?)?;
    frame.system("launch.register", crate::system::launch::register)?;
    frame.plan(
        "control.commands",
        crate::system::control::commands(crate::system::app::assembly::hooks()?)?,
    )?;
    frame.plan(
        "maintain.after",
        maintenance()?.map_error(|_| Fail::Publication),
    )?;
    frame.system(
        "instances.reap",
        crate::system::control::instance::schedule::reap,
    )?;
    frame.system("replies", crate::system::control::reply)?;
    frame.plan("instance.hooks", crate::system::launch::hooks::instance()?)?;
    frame.system("static", crate::system::control::startup)?;
    frame.system("retire.static", crate::system::control::retire_completed)?;
    frame.system("launch.completed", crate::system::launch::completed)?;
    frame.system("running", f::running)?;
    frame.system("activity", f::activity)?;
    frame.system("eligibility", crate::system::control::eligibility)?;
    frame.system("settle", f::settle)?;
    frame.system("idle", f::idle)?;
    frame.system("deadline", f::deadline)?;
    frame.system("ruin.rest", crate::system::control::ruin_rest)?;
    frame.system("done", f::done)?;
    frame.system("bound", f::bound)?;
    frame.system("stopping.bound", f::stopping_bound)?;
    frame.system(
        "instances.pending",
        crate::system::control::instance::schedule::pending,
    )?;
    frame.system("pending", crate::system::control::pending)?;
    frame.system("watch.entries", watch::entries)?;

    frame.system("watch.identity", watch::identity_changes)?;
    frame.system("watch.loader", crate::system::loader::watch)?;
    frame.system("watch.publication", watch::publication)?;
    frame.system("watch.construction", watch::construction)?;
    frame.system("watch.tasks", watch::tasks)?;
    frame.system("watch.connections", watch::connections)?;
    frame.system("watch.apply", watch::apply)?;
    frame.system("wait", watch::wait)?;
    frame.build()
}
pub fn shutdown() -> Result<Plan<Fail>, BuildError> {
    let mut stop = Schedule::sequence();
    stop.plan("loader.close", crate::system::loader::shutdown()?)?;
    stop.system("stopping", crate::system::app::life::stopping)?;
    stop.system("join", crate::system::app::life::join)?;
    stop.build()
}

use crate::system::app::Fault as Fail;

use ::schedule::{BuildError, Plan, Schedule};
pub fn maintenance() -> Result<Plan<Fail>, BuildError> {
    let mut plan = Schedule::sequence();
    let mut connections = Schedule::sequence();
    connections.system("connect", crate::system::operator::connect)?;
    plan.plan(
        "operator.connections",
        connections.build()?.map_error(|_| Fail::Dead),
    )?;
    plan.plan(
        "identity",
        crate::system::identity::maintenance()?.map_error(|_| Fail::Dead),
    )?;
    plan.system("identity.export", crate::system::identity::export::receive)?;
    plan.build()
}
pub fn startup() -> Result<Plan<&'static str>, BuildError> {
    use crate::system::app::boot;
    let mut start = Schedule::sequence();
    start.system("spawn", boot::spawn)?;
    start.system("embark", boot::embark)?;
    start.system("adopt", boot::adopt)?;
    start.system("operator.root", crate::system::app::operator::bootstrap)?;
    let mut identity_export = Schedule::sequence();
    identity_export.system("bootstrap", crate::system::identity::export::bootstrap)?;
    start.plan(
        "identity.export.bootstrap",
        identity_export
            .build()?
            .map_error(|_| "identity export bootstrap"),
    )?;
    start.system("identity", boot::identity)?;
    start.system("wire", boot::wire)?;
    start.system(
        "identity.namespace",
        crate::system::identity::namespace::prepare,
    )?;
    start.system(
        "control.namespace",
        crate::system::control::namespace::prepare,
    )?;
    start.system("loader", crate::system::loader::faces)?;
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
    frame.plan("maintenance", maintenance()?)?;
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
    frame.plan("launch", crate::system::launch::frame()?)?;
    frame.plan("assembly.jobs", crate::system::assembly::frame()?)?;
    frame.plan(
        "control.commands",
        crate::system::control::commands(crate::system::assembly::static_plan::hooks()?)?,
    )?;
    frame.system(
        "instances.reap",
        crate::system::control::instance::schedule::reap,
    )?;
    frame.system("replies", crate::system::control::reply)?;
    frame.plan(
        "instance.hooks",
        crate::system::assembly::hooks::instance()?,
    )?;
    frame.system("static", crate::system::control::startup)?;
    frame.system("retire.static", crate::system::control::retire_completed)?;
    frame.system("assembly.completed", crate::system::assembly::completed)?;
    frame.system("running", f::running)?;
    frame.system("activity", f::activity)?;
    frame.system("eligibility", crate::system::control::eligibility)?;
    // 声明的退休提供方全部结清后才进入 settling。
    frame.system("settle.open", f::gate_open)?;
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
    frame.system("watch.construction", watch::construction)?;
    frame.system("watch.tasks", watch::tasks)?;
    frame.system("watch.connections", watch::connections)?;
    frame.system("watch.apply", watch::apply)?;
    frame.system("wait", watch::wait)?;
    frame.build()
}
pub fn shutdown() -> Result<Plan<Fail>, BuildError> {
    let mut stop = Schedule::sequence();
    // **关闭顺序**：退场门（`policy::settle`）已经让 Hub 把全部 DMA 绑定结清，
    // 之后才撤镜像/供给/命名空间或走通用任务终止；不能先杀驱动器（那会把未确认的持有丢掉）。
    stop.plan("loader.close", crate::system::loader::shutdown()?)?;
    let mut supplies = Schedule::sequence();
    supplies.system("close", crate::system::app::supplies::shutdown)?;
    stop.plan("supplies.close", supplies.build()?)?;
    let mut resources = Schedule::sequence();
    resources.system(
        "control.namespace",
        crate::system::control::namespace::retire,
    )?;
    resources.system(
        "identity.namespace",
        crate::system::identity::namespace::retire,
    )?;
    stop.plan(
        "control.namespace",
        resources.build()?.map_error(|_| Fail::Dead),
    )?;
    stop.system("stopping", crate::system::app::life::stopping)?;
    stop.system("join", crate::system::app::life::join)?;
    stop.build()
}

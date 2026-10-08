use super::{
    Fail, answer, door, events, exchange, frame,
    management::{self, Ack},
};
use ::schedule::{Plan, Schedule};
use system_api::operator as ocall;
pub(super) fn plans() -> Result<[Plan<Fail>; 3], ::schedule::BuildError> {
    let mut start = Schedule::sequence();
    start.system("tip", management::tip)?;
    let mut hints = Schedule::sequence();
    hints.system("wired", management::wired)?;
    hints.system("guest", management::guest)?;
    hints.system("mutation", management::mutate)?;
    hints.system("changes", events::emit::<Ack>)?;
    hints.system("ack", management::acknowledge)?;
    let mut frame = Schedule::sequence();
    frame.system("health", frame::health)?;
    frame.system("query", frame::query)?;
    frame.system("hints.receive", management::receive_tips)?;
    frame.system("hints.budget", management::budget)?;
    frame.subplans(
        "hints",
        management::select,
        alloc::vec![((), hints.build()?)],
        management::finish,
    )?;
    frame.system("wait", exchange::wait)?;
    frame.system("guest.select", exchange::select_guest)?;
    frame.system("receive", exchange::receive)?;
    frame.system("validate", door::validate)?;
    frame.system("permit", door::permit)?;
    frame.system("authorize", door::authorize)?;
    frame.system("admit", door::admit)?;
    frame.system("subscribe", events::subscribe)?;
    frame.system("apply", answer::apply)?;
    frame.system("publish", events::emit::<ocall::Union>)?;
    frame.system("reply", exchange::reply)?;
    frame.system("sweep", exchange::sweep)?;
    let mut stop = Schedule::sequence();
    stop.system("close", management::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

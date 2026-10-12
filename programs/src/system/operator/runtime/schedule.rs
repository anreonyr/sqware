use super::{Fail, answer, authority, door, events, exchange, frame, management};
use ::schedule::{Plan, Schedule};
pub(super) fn plans() -> Result<[Plan<Fail>; 3], ::schedule::BuildError> {
    let mut start = Schedule::sequence();
    start.system("tip", management::tip)?;
    let mut hints = Schedule::sequence();
    hints.system("wired", management::wired)?;
    hints.system("guest", management::guest)?;
    hints.system("root", management::root)?;
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
    frame.system("authority", authority::handle)?;
    frame.system("guest.select", exchange::select_guest)?;
    frame.system("receive", exchange::receive)?;
    frame.system("validate", door::validate)?;
    frame.system("permit", door::permit)?;
    frame.system("authorize", door::authorize)?;
    frame.system("admit", door::admit)?;
    frame.system("subscribe", events::subscribe)?;
    frame.system("apply", answer::apply)?;
    frame.system("publish", events::emit)?;
    frame.system("reply", exchange::reply)?;
    frame.system("sweep", exchange::sweep)?;
    let mut stop = Schedule::sequence();
    stop.system("close", management::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

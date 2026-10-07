use ::schedule::{Plan, Schedule};
use super::{
    Fail, answer, door, frame, session,
    tip::{self, Ack},
    watch,
};
use protocol::{
    system::operator as ocall,
};
pub(super) fn plans() -> Result<[Plan<Fail>; 3], ::schedule::BuildError> {
    let mut start = Schedule::sequence();
    start.system("tip", tip::tip)?;
    let mut hints = Schedule::sequence();
    hints.system("wired", tip::wired)?;
    hints.system("guest", tip::guest)?;
    hints.system("mutation", tip::mutate)?;
    hints.system("changes", watch::emit::<Ack>)?;
    hints.system("ack", tip::acknowledge)?;
    let mut frame = Schedule::sequence();
    frame.system("health", frame::health)?;
    frame.system("query", frame::query)?;
    frame.system("hints.receive", tip::receive_tips)?;
    frame.system("hints.budget", tip::budget)?;
    frame.subplans("hints", tip::select, alloc::vec![((), hints.build()?)], tip::finish)?;
    frame.system("retry", session::retry)?;
    frame.system("arm", session::arm)?;
    frame.system("wait", session::wait)?;
    frame.system("guest.select", session::select_guest)?;
    frame.system("receive", session::receive)?;
    frame.system("validate", door::validate)?;
    frame.system("permit", door::permit)?;
    frame.system("authorize", door::authorize)?;
    frame.system("admit", door::admit)?;
    frame.system("subscribe", watch::subscribe)?;
    frame.system("apply", answer::apply)?;
    frame.system("publish", watch::emit::<ocall::Union>)?;
    frame.system("reply", session::reply)?;
    frame.system("sweep", session::sweep)?;
    let mut stop = Schedule::sequence();
    stop.system("close", tip::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

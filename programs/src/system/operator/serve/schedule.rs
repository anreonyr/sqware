use super::{
    Fail, answer, door, frame, session,
    tip::{self, Ack},
    watch,
};
use protocol::{
    common::schedule::{Plan, Schedule},
    system::operator as ocall,
};
pub(super) fn plans() -> Result<[Plan<Fail>; 3], protocol::common::schedule::BuildError> {
    let mut start = Schedule::new();
    start.add_system("tip", 0u8, tip::tip)?;
    let mut hints = Schedule::new();
    hints.add_system("wired", 0u8, tip::wired)?;
    hints.add_system("guest", 1, tip::guest)?;
    hints.add_system("mutation", 2, tip::mutate)?;
    hints.add_system("changes", 3, watch::emit::<Ack>)?;
    hints.add_system("ack", 4, tip::acknowledge)?;
    let mut frame = Schedule::new();
    frame.add_system("health", 0u8, frame::health)?;
    frame.add_system("query", 1, frame::query)?;
    frame.add_system("hints.receive", 2, tip::receive_tips)?;
    frame.add_system("hints.budget", 3, tip::budget)?;
    frame.add_subplans(
        "hints",
        4,
        tip::select,
        alloc::vec![((), hints.build()?)],
        tip::finish,
    )?;
    frame.add_system("retry", 5, session::retry)?;
    frame.add_system("arm", 6, session::arm)?;
    frame.add_system("wait", 7, session::wait)?;
    frame.add_system("guest.select", 8, session::select_guest)?;
    frame.add_system("receive", 9, session::receive)?;
    frame.add_system("validate", 10, door::validate)?;
    frame.add_system("permit", 11, door::permit)?;
    frame.add_system("authorize", 12, door::authorize)?;
    frame.add_system("admit", 13, door::admit)?;
    frame.add_system("subscribe", 14, watch::subscribe)?;
    frame.add_system("apply", 15, answer::apply)?;
    frame.add_system("publish", 16, watch::emit::<ocall::Union>)?;
    frame.add_system("reply", 17, session::reply)?;
    frame.add_system("sweep", 18, session::sweep)?;
    let mut stop = Schedule::new();
    stop.add_system("close", 0u8, tip::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

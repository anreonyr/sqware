use super::{Fail, book, face, frame};
use protocol::common::schedule::{Plan, Schedule};
pub(super) fn plans() -> Result<[Plan<Fail>; 3], protocol::common::schedule::BuildError> {
    let mut start = Schedule::new();
    start.add_system("book", 0u8, book::initialize)?;
    start.add_system("faces", 1, face::faces)?;
    let mut request = Schedule::new();
    request.add_system("apply", 0u8, book::apply)?;
    request.add_system("epoch", 1, super::revision::publish)?;
    request.add_system("notify", 2, super::revision::notify)?;
    request.add_system("reply", 3, face::reply)?;
    let mut frame = Schedule::new();
    frame.add_system("health", 0u8, frame::health)?;
    frame.add_system("wait", 1, face::wait)?;
    frame.add_system("receive", 2, face::receive)?;
    frame.add_system("budget", 3, face::budget)?;
    frame.add_subplans(
        "requests",
        4,
        face::select,
        alloc::vec![((), request.build()?)],
        face::finish,
    )?;
    let mut stop = Schedule::new();
    stop.add_system("close", 0u8, face::close)?;
    stop.add_system("changes.close", 1, super::revision::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

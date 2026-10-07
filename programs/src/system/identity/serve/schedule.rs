use super::{Fail, book, face, frame};
use ::schedule::{Plan, Schedule};
pub(super) fn plans() -> Result<[Plan<Fail>; 3], ::schedule::BuildError> {
    let mut start = Schedule::sequence();
    start.system("book", book::initialize)?;
    start.system("faces", face::faces)?;
    let mut request = Schedule::sequence();
    request.system("apply", book::apply)?;
    request.system("epoch", super::revision::publish)?;
    request.system("notify", super::revision::notify)?;
    request.system("reply", face::reply)?;
    let mut frame = Schedule::sequence();
    frame.system("health", frame::health)?;
    frame.system("wait", face::wait)?;
    frame.system("receive", face::receive)?;
    frame.system("budget", face::budget)?;
    frame.subplans("requests", face::select, alloc::vec![((), request.build()?)], face::finish)?;
    let mut stop = Schedule::sequence();
    stop.system("close", face::close)?;
    stop.system("changes.close", super::revision::close)?;
    Ok([start.build()?, frame.build()?, stop.build()?])
}

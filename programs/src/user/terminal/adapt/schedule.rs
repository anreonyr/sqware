use super::{frame, server};
use runtime::schedule::{BuildError, Plan, Schedule};

pub(super) fn frame() -> Result<Plan<env::Reason>, BuildError> {
    let mut frame = Schedule::new();
    frame.add_system("requests", 0u8, server::requests)?;
    frame.add_system("output", 1, frame::output)?;
    frame.add_system("output.flush", 2, frame::flush)?;
    frame.add_system("receive", 3, frame::receive)?;
    frame.add_system("feed", 4, frame::feed)?;
    frame.add_system("echo.flush", 5, frame::flush)?;
    frame.add_system("deliver", 6, server::deliver)?;
    frame.add_system("wait", 7, server::wait)?;
    frame.build()
}

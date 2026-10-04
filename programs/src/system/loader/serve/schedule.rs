use crate::system::control::serve::Fail;
use protocol::common::schedule::{BuildError, Plan, Schedule};
pub fn frame() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::new();
    schedule.add_system("receive", 0u8, super::answer::receive)?;
    schedule.add_system("settle", 1, super::frame::settle)?;
    schedule.add_system("build", 2, super::frame::build)?;
    schedule.build()
}
pub fn shutdown() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::new();
    schedule.add_system("withdraw", 0u8, super::publication::withdraw)?;
    schedule.add_system("close", 1, super::frame::close)?;
    schedule.build()
}

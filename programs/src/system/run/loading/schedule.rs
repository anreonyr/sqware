use crate::system::control::serve::Fail;
use ::schedule::{BuildError, Plan, Schedule};
pub fn frame() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::sequence();
    schedule.system("receive", super::answer::receive)?;
    schedule.system("settle", super::frame::settle)?;
    schedule.system("build", super::frame::build)?;
    schedule.build()
}
pub fn shutdown() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::sequence();
    schedule.system("withdraw", super::publication::withdraw)?;
    schedule.system("close", super::frame::close)?;
    schedule.build()
}

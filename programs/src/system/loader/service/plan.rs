use crate::system::app::Fault as Fail;
use ::schedule::{BuildError, Plan, Schedule};
pub(crate) fn frame() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::sequence();
    schedule.system("receive", super::answer::receive)?;
    schedule.build()
}
pub(crate) fn shutdown() -> Result<Plan<Fail>, BuildError> {
    let mut schedule = Schedule::sequence();
    schedule.system("withdraw", super::publication::withdraw)?;
    schedule.system("close", super::execution::close)?;
    schedule.build()
}

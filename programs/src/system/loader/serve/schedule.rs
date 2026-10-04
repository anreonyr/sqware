use protocol::common::schedule::{BuildError, Plan, Schedule};
pub fn frame() -> Result<Plan<crate::system::control::serve::Fail>, BuildError> {
    let mut schedule = Schedule::new();
    schedule.add_system("receive", 0u8, super::answer::receive)?;
    schedule.add_system("build", 1, super::frame::build)?;
    schedule.build()
}

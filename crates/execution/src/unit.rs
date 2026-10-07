//! Task creation and domain-local closure execution.

pub mod join;
mod tls;

use env::{TaskId, TeamId, UnitResult, VirtAddr};

/// Create a held task with scalar startup arguments.
pub fn spawn(team: TeamId, entry: usize, args: &[usize], stack: usize) -> UnitResult<TaskId> {
    env::unit::spawn(
        team,
        entry,
        VirtAddr::new(args.as_ptr() as usize),
        args.len(),
        stack,
    )
}

pub(crate) unsafe fn bootstrap() {
    unsafe { tls::bootstrap() }
}

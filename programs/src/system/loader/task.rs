use alloc::vec::Vec;
use env::{PieToken, TaskId, TeamId, UnitResult, pie, unit};

pub struct Unit {
    pub(in crate::system::loader) team: TeamId,
    pub(in crate::system::loader) entry: usize,
    pub(in crate::system::loader) private: Vec<PieToken>,
    pub(in crate::system::loader) committed: bool,
}

impl Unit {
    pub fn team(&self) -> TeamId {
        self.team
    }

    pub fn spawn(mut self, args: &[usize], stack: usize) -> UnitResult<TaskId> {
        let task = execution::unit::spawn(self.team, self.entry, args, stack)?;
        self.committed = true;
        Ok(task)
    }
}

impl Drop for Unit {
    fn drop(&mut self) {
        if !self.committed {
            let _ = unit::oust(self.team);
            for token in self.private.drain(..) {
                let _ = pie::release(token, env::ReleaseMode::Revoke);
            }
        }
    }
}

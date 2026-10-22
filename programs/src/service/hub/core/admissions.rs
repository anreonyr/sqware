use alloc::vec::Vec;

use env::TaskId;
use hub_api::{Close, Fail};

use super::{Admission, Ledger, Step};

const ADMISSIONS: usize = 256;

/// Closed launch identities are retained long enough to reject a late replay.
const CLOSED_ADMISSIONS: usize = 32;

impl Ledger {
    pub fn reserve_admission(&mut self) -> Result<(), Fail> {
        if self.admissions.len() >= ADMISSIONS {
            return Err(Fail::Full);
        }
        self.admissions.try_reserve(1).map_err(|_| Fail::Full)
    }

    /// Record the same instance once. A settled admission never reopens.
    pub fn admit(&mut self, task: TaskId, deadline: u64) -> Result<(), Fail> {
        if task.get() == 0 {
            return Err(Fail::Denied);
        }
        if self.closed_admissions.contains(&task) {
            return Err(Fail::Denied);
        }
        if let Some(admission) = self.admission(task) {
            return if admission.closed() {
                Err(Fail::Denied)
            } else {
                Ok(())
            };
        }
        if self.settling {
            return Err(Fail::Denied);
        }
        self.reserve_admission()?;
        self.admissions.push(Admission::new(task, deadline));
        Ok(())
    }

    pub fn admission(&self, task: TaskId) -> Option<&Admission> {
        self.admissions.iter().find(|admission| admission.task == task)
    }

    pub fn admission_mut(&mut self, task: TaskId) -> Option<&mut Admission> {
        self.admissions
            .iter_mut()
            .find(|admission| admission.task == task)
    }

    pub fn admissions(&self) -> impl Iterator<Item = &Admission> + '_ {
        self.admissions.iter()
    }

    pub fn admissions_pending(&self) -> bool {
        !self.admissions.is_empty()
    }

    /// Unclaimed admissions keep their original deadline, then close for retirement.
    pub fn expire_admissions(&mut self, now: u64, mut enqueue: impl FnMut(TaskId)) {
        for admission in &mut self.admissions {
            if admission.expired(now) {
                admission.close();
                enqueue(admission.task);
            }
        }
    }

    /// A retirement target is the proof that its execution domain stopped.
    ///
    /// Close every bound responsibility and move it past `StopCpu` exactly once. No
    /// admission means there is no Hub execution lifecycle to finish.
    pub fn stop_target(&mut self, task: TaskId) -> usize {
        if let Some(admission) = self.admission_mut(task) {
            admission.close();
        }
        for binding in &mut self.bindings {
            if binding.open() && binding.execution == task {
                binding.close(Close::Normal);
                if binding.step == Step::StopCpu {
                    binding.step = Step::Invalidate;
                }
            }
        }
        self.open_bindings_for(task)
    }

    pub fn open_bindings_for(&self, task: TaskId) -> usize {
        self.bindings
            .iter()
            .filter(|binding| binding.open() && binding.execution == task)
            .count()
    }

    /// Release authority returned to Control: forget the admission and remember the
    /// closed identity so a late launch report cannot revive it.
    pub fn released(&mut self, task: TaskId) {
        if let Some(at) = self
            .admissions
            .iter()
            .position(|admission| admission.task == task)
        {
            self.admissions.remove(at);
        }
        if self.closed_admissions.contains(&task) {
            return;
        }
        if self.closed_admissions.len() >= CLOSED_ADMISSIONS {
            self.closed_admissions.remove(0);
        }
        self.closed_admissions.push(task);
    }

    pub fn admission_closed(&self, task: TaskId) -> bool {
        self.closed_admissions.contains(&task)
    }

    pub fn closed_admission_count(&self) -> usize {
        self.closed_admissions.len()
    }
}

impl Ledger {
    pub fn admission_tasks(&self) -> Vec<TaskId> {
        self.admissions.iter().map(|admission| admission.task).collect()
    }
}

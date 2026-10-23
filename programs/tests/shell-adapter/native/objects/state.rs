use super::LocalPort;
use crate::core::JobId;

pub struct Objects {
    pub used: usize,
    pub ports: Vec<Option<LocalPort>>,
    pub jobs: Vec<JobId>,
}

impl Objects {
    pub fn new() -> Self {
        Self {
            used: 0,
            ports: Vec::new(),
            jobs: Vec::new(),
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn port_mut(&mut self, id: u64) -> Option<&mut LocalPort> {
        self.ports.get_mut(id as usize).and_then(Option::as_mut)
    }

    pub fn port(&self, id: u64) -> Option<&LocalPort> {
        self.ports.get(id as usize).and_then(Option::as_ref)
    }

    pub fn has_job(&self, id: JobId) -> bool {
        self.jobs.contains(&id)
    }
}

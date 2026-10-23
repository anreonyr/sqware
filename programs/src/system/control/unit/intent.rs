use super::{Control, verdict::Fail};

impl Control {
    /// Record that an accepted lifecycle action wants this target running.
    pub(crate) fn request_run(&mut self, name: &str) -> Result<(), Fail> {
        self.table.set_run_requested(name, true)
    }

    /// Record that an accepted lifecycle action stopped wanting this target.
    pub(crate) fn request_stop(&mut self, name: &str) -> Result<(), Fail> {
        self.table.set_run_requested(name, false)
    }

    pub(crate) fn run_requested(&self, name: &str) -> Option<bool> {
        self.table.find(name).map(|row| row.run_requested)
    }
}

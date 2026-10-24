use super::*;

impl Controller {
    pub fn test_push_job(&mut self, job: RuntimeJob) {
        self.jobs.push(job);
    }
}

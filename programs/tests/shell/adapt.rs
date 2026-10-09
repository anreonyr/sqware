#[path = "../../src/user/shell/adapt/ports.rs"]
pub mod ports;
pub mod controller {
    use crate::{core::*, native::objects::Objects};
    use std::collections::BTreeMap;
    pub struct RuntimeJob {
        pub model: Job,
        pub error: Option<String>,
    }
    pub struct Controller {
        pub catalogue: shell_api::Catalogue,
        pub foreground: Option<JobId>,
        jobs: BTreeMap<JobId, RuntimeJob>,
        pub events: usize,
        pub finish: bool,
    }
    impl Controller {
        pub fn new() -> Self {
            Self {
                catalogue: shell_api::Catalogue {
                    images: vec![shell_api::Image {
                        name: "mock".into(),
                        seed: abi::PieToken::NONE,
                        length: 1,
                        ports: vec![],
                    }],
                },
                foreground: None,
                jobs: BTreeMap::new(),
                events: 0,
                finish: false,
            }
        }
        pub fn job(&self, id: JobId) -> Option<&RuntimeJob> {
            self.jobs.get(&id)
        }
        pub fn jobs(&self) -> impl Iterator<Item = JobId> + '_ {
            self.jobs.keys().copied()
        }
        pub fn prepare(
            &mut self,
            plan: Plan,
            _: lisp::RootValue,
            _: &mut Objects,
        ) -> Result<JobId, &'static str> {
            let id = JobId(self.jobs.len() as u64 + 1);
            self.jobs.insert(
                id,
                RuntimeJob {
                    model: Job::prepare(id, plan.commands.len()),
                    error: None,
                },
            );
            Ok(id)
        }
        pub fn start(&mut self, id: JobId) -> Result<(), &'static str> {
            self.jobs
                .get_mut(&id)
                .unwrap()
                .model
                .start()
                .map_err(|_| "start")
        }
        pub fn pause(&mut self, id: JobId, reason: PauseReason) -> Result<(), &'static str> {
            self.jobs
                .get_mut(&id)
                .unwrap()
                .model
                .pause(reason)
                .map_err(|_| "pause")
        }
        pub fn resume(&mut self, id: JobId) -> Result<(), &'static str> {
            self.jobs
                .get_mut(&id)
                .unwrap()
                .model
                .resume()
                .map_err(|_| "resume")
        }
        pub fn cancel(&mut self, id: JobId) -> Result<(), &'static str> {
            self.jobs.get_mut(&id).unwrap().model.cancel();
            Ok(())
        }
        pub fn foreground(&mut self, id: JobId) -> Result<(), &'static str> {
            self.jobs.get_mut(&id).unwrap().model.foreground = true;
            self.foreground = Some(id);
            Ok(())
        }
        pub fn background(&mut self, id: JobId) -> Result<(), &'static str> {
            self.jobs.get_mut(&id).unwrap().model.foreground = false;
            self.foreground = None;
            Ok(())
        }
        pub fn advance(&mut self, _: &mut Objects) {
            self.events += 1;
            if self.events % 3 != 0 {
                return;
            }
            for job in self.jobs.values_mut() {
                match job.model.status {
                    JobStatus::Preparing => {
                        job.model.prepared().unwrap();
                    }
                    JobStatus::Starting | JobStatus::Pausing | JobStatus::Resuming => {
                        if let Some(member) = job.model.pending_member() {
                            job.model.acknowledge(member).unwrap();
                        }
                    }
                    JobStatus::Running if self.finish => {
                        for index in 0..job.model.results.len() {
                            job.model.complete(index, index * 17).unwrap();
                        }
                        job.model.drained();
                    }
                    JobStatus::Cancelling => {
                        for index in 0..job.model.results.len() {
                            job.model.skip(index).unwrap();
                        }
                        job.model.drained();
                    }
                    _ => {}
                }
            }
        }
        pub fn reclaim(&mut self, objects: &Objects) {
            self.jobs
                .retain(|id, job| !job.model.status.terminal() || objects.has_job(*id));
        }
    }
}

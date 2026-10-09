use alloc::{vec, vec::Vec};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct JobId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Preparing,
    Prepared,
    Starting,
    Running,
    Pausing,
    Paused,
    Resuming,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}
impl JobStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Prepared => "prepared",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Resuming => "resuming",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    Requested,
    BackgroundRead,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobFail {
    Transition,
    Missing,
    Full,
    System,
    Timeout,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberResult {
    Exited(usize),
    NotStarted,
}
#[derive(Clone, Debug)]
pub enum JobEvent {
    Completed(Vec<MemberResult>),
    Paused(PauseReason),
    Pending,
}
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    pub status: JobStatus,
    pub foreground: bool,
    pub results: Vec<Option<MemberResult>>,
    pub pause_reason: PauseReason,
    acknowledged: Vec<bool>,
    drained: bool,
    failed: bool,
}
impl Job {
    pub fn prepare(id: JobId, members: usize) -> Self {
        Self {
            id,
            status: JobStatus::Preparing,
            foreground: false,
            results: vec![None; members],
            acknowledged: vec![false; members],
            drained: false,
            failed: false,
            pause_reason: PauseReason::Requested,
        }
    }
    pub fn prepared(&mut self) -> Result<(), JobFail> {
        if self.status != JobStatus::Preparing {
            return Err(JobFail::Transition);
        }
        self.status = JobStatus::Prepared;
        Ok(())
    }
    fn begin(&mut self, status: JobStatus) {
        self.status = status;
        self.acknowledged.fill(false);
    }
    pub fn start(&mut self) -> Result<(), JobFail> {
        if self.status != JobStatus::Prepared {
            return Err(JobFail::Transition);
        }
        self.begin(JobStatus::Starting);
        Ok(())
    }
    pub fn pause(&mut self, reason: PauseReason) -> Result<(), JobFail> {
        if self.status == JobStatus::Paused || self.status == JobStatus::Pausing {
            return Ok(());
        }
        if !matches!(
            self.status,
            JobStatus::Running | JobStatus::Starting | JobStatus::Resuming
        ) {
            return Err(JobFail::Transition);
        }
        self.pause_reason = reason;
        self.begin(JobStatus::Pausing);
        Ok(())
    }
    pub fn resume(&mut self) -> Result<(), JobFail> {
        if self.status == JobStatus::Running {
            return Ok(());
        }
        if self.status != JobStatus::Paused {
            return Err(JobFail::Transition);
        }
        self.begin(JobStatus::Resuming);
        Ok(())
    }
    pub fn cancel(&mut self) {
        if !self.status.terminal() {
            self.begin(JobStatus::Cancelling);
        }
    }
    pub fn fail(&mut self) {
        self.failed = true;
        self.cancel();
    }
    pub fn acknowledge(&mut self, member: usize) -> Result<(), JobFail> {
        *self.acknowledged.get_mut(member).ok_or(JobFail::Missing)? = true;
        self.advance();
        Ok(())
    }
    pub fn complete(&mut self, member: usize, reason: usize) -> Result<(), JobFail> {
        self.results
            .get_mut(member)
            .ok_or(JobFail::Missing)?
            .get_or_insert(MemberResult::Exited(reason));
        self.advance();
        Ok(())
    }
    pub fn skip(&mut self, member: usize) -> Result<(), JobFail> {
        self.results
            .get_mut(member)
            .ok_or(JobFail::Missing)?
            .get_or_insert(MemberResult::NotStarted);
        self.advance();
        Ok(())
    }
    pub fn pending_member(&self) -> Option<usize> {
        self.acknowledged
            .iter()
            .zip(&self.results)
            .position(|(ack, result)| !*ack && result.is_none())
    }
    pub fn drained(&mut self) {
        self.drained = true;
        self.advance();
    }
    pub fn advance(&mut self) {
        let complete = self.results.iter().all(Option::is_some);
        let confirmed = self
            .acknowledged
            .iter()
            .zip(&self.results)
            .all(|(ack, result)| *ack || result.is_some());
        self.status = match self.status {
            JobStatus::Starting | JobStatus::Resuming if confirmed => JobStatus::Running,
            JobStatus::Pausing if confirmed => JobStatus::Paused,
            JobStatus::Cancelling if complete && self.drained => {
                if self.failed {
                    JobStatus::Failed
                } else {
                    JobStatus::Cancelled
                }
            }
            other => other,
        };
        if matches!(
            self.status,
            JobStatus::Running | JobStatus::Pausing | JobStatus::Paused
        ) && complete
            && self.drained
        {
            self.status = JobStatus::Completed;
        }
        if self.status.terminal() || self.status == JobStatus::Paused {
            self.foreground = false;
        }
    }
    pub fn event(&self) -> JobEvent {
        if self.status.terminal() {
            JobEvent::Completed(self.results.iter().filter_map(|result| *result).collect())
        } else if self.status == JobStatus::Paused {
            JobEvent::Paused(self.pause_reason)
        } else {
            JobEvent::Pending
        }
    }
}

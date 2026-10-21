use env::{PieToken, TaskId, TeamId};

use super::{ExecutionEffects, ExecutionFailure, ExecutionStep};

pub struct Execution {
    pub task: TaskId,
    pub team: Option<TeamId>,
    pub startup: Option<PieToken>,
    pub step: ExecutionStep,
    pub deadline: u64,
    pub failure: Option<ExecutionFailure>,
    pub(super) claim_high: u64,
    retiring: bool,
}

impl Execution {
    pub fn new(task: TaskId, deadline: u64) -> Self {
        Self {
            task,
            team: None,
            startup: None,
            step: ExecutionStep::Identify,
            deadline,
            failure: None,
            claim_high: 0,
            retiring: false,
        }
    }

    pub fn retire(&mut self) {
        self.retiring = true;
        if self.team.is_some()
            && !matches!(
                self.step,
                ExecutionStep::Stop | ExecutionStep::Oust | ExecutionStep::Release | ExecutionStep::Done
            )
        {
            self.step = ExecutionStep::Stop;
        }
    }

    pub fn stopped(&self) -> bool {
        self.team.is_some()
            && matches!(self.step, ExecutionStep::Release | ExecutionStep::Done)
    }

    pub fn pending(&self) -> bool {
        !matches!(self.step, ExecutionStep::Running | ExecutionStep::Done)
    }

    pub fn advance(
        &mut self,
        now: u64,
        effects: &mut impl ExecutionEffects,
    ) -> Result<(), ExecutionFailure> {
        if now >= self.deadline && self.step != ExecutionStep::Running {
            self.retire();
        }
        let result = self.advance_step(effects);
        if let Err(failure) = result {
            self.failure.get_or_insert(failure);
        }
        result
    }

    fn advance_step(
        &mut self,
        effects: &mut impl ExecutionEffects,
    ) -> Result<(), ExecutionFailure> {
        use ExecutionStep as Step;
        match self.step {
            Step::Identify => match effects.team(self.task)? {
                Some(team) => {
                    self.team = Some(team);
                    self.step = if self.retiring { Step::Stop } else { Step::Create };
                }
                None => self.step = Step::Done,
            },
            Step::Create => {
                self.startup = Some(effects.create()?);
                self.step = Step::Populate;
            }
            Step::Populate => {
                effects.populate(self.startup.expect("startup hole"))?;
                self.step = Step::Deliver;
            }
            Step::Deliver => {
                effects.deliver(self.startup.expect("startup hole"), self.task)?;
                self.step = Step::Restrict;
            }
            Step::Restrict => {
                effects.restrict(self.startup.expect("startup hole"))?;
                self.step = Step::Embark;
            }
            Step::Embark => {
                effects.embark(self.task)?;
                self.step = Step::Running;
            }
            Step::Running => {
                if effects.exited(self.task)? {
                    self.retire();
                }
            }
            Step::Stop => {
                effects.stop(self.team.expect("identified team"))?;
                self.step = Step::Oust;
            }
            Step::Oust => {
                if effects.oust(self.team.expect("identified team"))? {
                    self.step = Step::Release;
                }
            }
            Step::Release => {
                if let Some(startup) = self.startup {
                    effects.release(startup)?;
                    self.startup = None;
                }
                self.step = Step::Done;
            }
            Step::Done => {}
        }
        Ok(())
    }
}

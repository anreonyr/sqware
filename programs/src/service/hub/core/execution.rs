use env::{TaskId, TeamId};

use super::{ExecutionEffects, ExecutionFailure, ExecutionStep};

pub struct Execution {
    pub task: TaskId,
    pub team: Option<TeamId>,
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
                ExecutionStep::Stop | ExecutionStep::Oust | ExecutionStep::Done
            )
        {
            self.step = ExecutionStep::Stop;
        }
    }

    pub fn stopped(&self) -> bool {
        self.team.is_some() && self.step == ExecutionStep::Done
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
                    self.step = if self.retiring { Step::Stop } else { Step::Embark };
                }
                None => self.step = Step::Done,
            },
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
                    self.step = Step::Done;
                }
            }
            Step::Done => {}
        }
        Ok(())
    }
}

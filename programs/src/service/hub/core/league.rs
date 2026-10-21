use alloc::string::String;
use system_api::identity::{CoalitionId, FoundId};
use super::{EnrollmentEffects, league_step::LeagueStep};

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct League {
    pub class: String,
    pub creation: FoundId,
    pub step: LeagueStep,
}

impl League {
    pub fn advance<E: EnrollmentEffects>(&mut self, effects: &mut E) -> Result<CoalitionId, E::Error> {
        loop {
            self.step = match self.step {
                LeagueStep::Found => LeagueStep::Admit(effects.found(self.creation)?),
                LeagueStep::Admit(coalition) => {
                    effects.admit(coalition)?;
                    LeagueStep::Activate(coalition)
                }
                LeagueStep::Activate(coalition) => {
                    effects.activate(coalition)?;
                    LeagueStep::Ready(coalition)
                }
                LeagueStep::Ready(coalition) => return Ok(coalition),
            };
        }
    }
}

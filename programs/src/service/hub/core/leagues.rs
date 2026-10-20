use alloc::string::String;
use hub_api::{Fail, INVENTORY_MAX};
use system_api::identity::{CoalitionId, FoundId};

use super::{EnrollmentEffects, Ledger, league::League, league_step::LeagueStep};

impl Ledger {
    pub fn prepare_league(&mut self, class: String) -> Result<(), Fail> {
        if self.leagues.iter().any(|league| league.class == class) {
            return Ok(());
        }
        if self.settling {
            return Err(Fail::Denied);
        }
        if self.leagues.len() >= INVENTORY_MAX {
            return Err(Fail::Full);
        }
        self.leagues.try_reserve(1).map_err(|_| Fail::Full)?;
        let creation = FoundId { value: self.leagues.len() as u64 + 1 };
        self.leagues.push(League { class, creation, step: LeagueStep::Found });
        Ok(())
    }

    pub fn advance_league<E: EnrollmentEffects>(
        &mut self,
        class: &str,
        effects: &mut E,
    ) -> Option<Result<CoalitionId, E::Error>> {
        if self.settling {
            return None;
        }
        self.leagues.iter_mut().find(|league| league.class == class)
            .map(|league| league.advance(effects))
    }

    pub fn coalition_of(&self, class: String) -> Option<CoalitionId> {
        self.leagues.iter().find_map(|league| match league.step {
            LeagueStep::Ready(coalition) if league.class == class => Some(coalition),
            _ => None,
        })
    }
}

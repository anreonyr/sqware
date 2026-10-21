use env::{TeamId, UnitFail, unit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receipt {
    Owner(TeamId),
    Returned(TeamId),
    Consumed,
}

impl Receipt {
    pub(crate) fn team(&self) -> Option<TeamId> {
        match self {
            Self::Owner(team) | Self::Returned(team) => Some(*team),
            Self::Consumed => None,
        }
    }

    pub(crate) fn consume(&mut self) -> Result<bool, UnitFail> {
        let Some(team) = self.team() else { return Ok(false) };
        unit::oust(team).map_err(|error| error.source)?;
        *self = Self::Consumed;
        Ok(true)
    }
}

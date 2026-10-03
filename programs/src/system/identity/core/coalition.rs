use env::TaskId;
use protocol::system::identity::{CoalitionId, Fail, PrincipalId, limits};

use super::IdentityBook;

pub(super) struct CoalitionNode {
    pub(super) manager: PrincipalId,
}

impl IdentityBook {
    pub(super) fn coalition(&self, id: CoalitionId) -> Result<usize, Fail> {
        if id.authority != self.authority { return Err(Fail::WrongAuthority); }
        let index = usize::try_from(id.slot).map_err(|_| Fail::UnknownCoalition)?;
        if index >= self.coalitions.len() { return Err(Fail::UnknownCoalition); }
        Ok(index)
    }

    pub fn found(&mut self, from: TaskId) -> Result<CoalitionId, Fail> {
        let manager = self.resolve(from).ok_or(Fail::Denied)?.current.principal;
        if self.coalitions.len() >= limits::MAX_COALITIONS
            || self.coalitions.iter().filter(|n| n.manager == manager).count()
                >= limits::MAX_CREATED_PER_PRINCIPAL
        { return Err(Fail::Full); }
        let slot = u64::try_from(self.coalitions.len()).map_err(|_| Fail::Full)?;
        slot.checked_add(1).ok_or(Fail::Full)?;
        self.coalitions.try_reserve(1).map_err(|_| Fail::Full)?;
        self.coalitions.push(CoalitionNode { manager });
        Ok(CoalitionId::new(self.authority, slot))
    }
}

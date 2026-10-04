use env::TaskId;
use protocol::system::identity::{Fail, PrincipalId, limits};

use super::IdentityBook;

pub(super) struct PrincipalNode {
    pub(super) parent: Option<PrincipalId>,
}

impl IdentityBook {
    pub fn root(&self) -> PrincipalId {
        PrincipalId::root(self.authority)
    }

    pub(super) fn principal(&self, id: PrincipalId) -> Result<usize, Fail> {
        if id.authority != self.authority {
            return Err(Fail::WrongAuthority);
        }
        let index = usize::try_from(id.slot).map_err(|_| Fail::UnknownPrincipal)?;
        if index >= self.principals.len() {
            return Err(Fail::UnknownPrincipal);
        }
        Ok(index)
    }

    pub fn sire(&self, p: PrincipalId) -> Result<Option<PrincipalId>, Fail> {
        Ok(self.principals[self.principal(p)?].parent)
    }

    pub fn heir(&self, ancestor: PrincipalId, descendant: PrincipalId) -> Result<bool, Fail> {
        self.principal(ancestor)?;
        let mut at = Some(descendant);
        while let Some(p) = at {
            if p == ancestor {
                return Ok(true);
            }
            at = self.principals[self.principal(p)?].parent;
        }
        Ok(false)
    }

    pub fn derive(&mut self, from: TaskId, parent: PrincipalId) -> Result<PrincipalId, Fail> {
        self.principal(parent)?;
        if from != self.installer && self.resolve(from).map(|b| b.current.principal) != Some(parent)
        {
            return Err(Fail::Denied);
        }
        if self.principals.len() >= limits::MAX_PRINCIPALS
            || self
                .principals
                .iter()
                .filter(|n| n.parent == Some(parent))
                .count()
                >= limits::MAX_CREATED_PER_PRINCIPAL
        {
            return Err(Fail::Full);
        }
        let slot = u64::try_from(self.principals.len()).map_err(|_| Fail::Full)?;
        slot.checked_add(1).ok_or(Fail::Full)?;
        self.principals.try_reserve(1).map_err(|_| Fail::Full)?;
        self.principals.push(PrincipalNode {
            parent: Some(parent),
        });
        Ok(PrincipalId::new(self.authority, slot))
    }
}

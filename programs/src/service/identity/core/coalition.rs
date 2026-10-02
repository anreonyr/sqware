use env::TaskId;
use protocol::service::identity::{CoalitionId, Cursor, Fail, Page, PageTarget, PrincipalId, limits};

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

    pub fn amid(&self, p: PrincipalId, c: CoalitionId) -> Result<bool, Fail> {
        self.principal(p)?;
        self.coalition(c)?;
        Ok(self.memberships.contains(&(p, c)))
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
        Ok(CoalitionId { authority: self.authority, slot })
    }
    fn manage(&self, from: TaskId, c: CoalitionId, p: PrincipalId) -> Result<(), Fail> {
        let index = self.coalition(c)?;
        self.principal(p)?;
        if from == self.installer { return Ok(()); }
        let sender = self.resolve(from).ok_or(Fail::Denied)?.current.principal;
        if sender != self.coalitions[index].manager { return Err(Fail::NotManager); }
        Ok(())
    }
    pub fn admit(&mut self, from: TaskId, c: CoalitionId, p: PrincipalId) -> Result<(), Fail> {
        self.manage(from, c, p)?;
        if self.memberships.contains(&(p, c)) { return Ok(()); }
        if self.memberships.len() >= limits::MAX_MEMBERSHIPS { return Err(Fail::Full); }
        let revision = self.revision.checked_add(1).ok_or(Fail::Full)?;
        self.memberships.try_reserve(1).map_err(|_| Fail::Full)?;
        self.memberships.push((p, c));
        self.revision = revision;
        Ok(())
    }
    pub fn expel(&mut self, from: TaskId, c: CoalitionId, p: PrincipalId) -> Result<(), Fail> {
        self.manage(from, c, p)?;
        let exists = self.memberships.contains(&(p, c));
        let revision = if exists {
            self.revision.checked_add(1).ok_or(Fail::Full)?
        } else { self.revision };
        self.memberships.retain(|m| *m != (p, c));
        for b in &mut self.bindings {
            if b.binding.origin.principal == p { b.binding.origin.coalitions.remove(c); }
            if b.binding.current.principal == p { b.binding.current.coalitions.remove(c); }
            let current = b.binding.current.coalitions;
            for &selected in current.as_slice() {
                if !b.binding.origin.coalitions.contains(selected) {
                    b.binding.current.coalitions.remove(selected);
                }
            }
        }
        self.revision = revision;
        Ok(())
    }
    fn cursor(&self, target: PageTarget, cursor: Option<Cursor>) -> Result<Option<u64>, Fail> {
        match cursor {
            None => Ok(None),
            Some(c) if c.target != target => Err(Fail::Bad),
            Some(c) if c.revision != self.revision => Err(Fail::Changed),
            Some(c) => Ok(Some(c.after)),
        }
    }
    pub fn members(&self, c: CoalitionId, cursor: Option<Cursor>) -> Result<Page<PrincipalId>, Fail> {
        self.coalition(c)?;
        let target = PageTarget::Members(c);
        let after = self.cursor(target, cursor)?;
        let mut ids = [self.root(); protocol::service::identity::limits::PAGE_ITEMS];
        let mut count = 0;
        let mut more = false;
        for slot in 0..self.principals.len() {
            let p = PrincipalId { authority: self.authority, slot: slot as u64 };
            if after.is_none_or(|a| p.slot > a) && self.memberships.contains(&(p, c)) {
                if count == ids.len() { more = true; break; }
                ids[count] = p;
                count += 1;
            }
        }
        let next = if more { Some(Cursor {
            target, revision: self.revision, after: ids[count - 1].slot,
        }) } else { None };
        Page::new(&ids[..count], next)
    }
    pub fn memberships(&self, p: PrincipalId, cursor: Option<Cursor>) -> Result<Page<CoalitionId>, Fail> {
        self.principal(p)?;
        let target = PageTarget::Memberships(p);
        let after = self.cursor(target, cursor)?;
        let zero = CoalitionId { authority: self.authority, slot: 0 };
        let mut ids = [zero; protocol::service::identity::limits::PAGE_ITEMS];
        let mut count = 0;
        let mut more = false;
        for slot in 0..self.coalitions.len() {
            let c = CoalitionId { authority: self.authority, slot: slot as u64 };
            if after.is_none_or(|a| c.slot > a) && self.memberships.contains(&(p, c)) {
                if count == ids.len() { more = true; break; }
                ids[count] = c;
                count += 1;
            }
        }
        let next = if more { Some(Cursor {
            target, revision: self.revision, after: ids[count - 1].slot,
        }) } else { None };
        Page::new(&ids[..count], next)
    }
}

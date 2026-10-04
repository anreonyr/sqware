//! One validated authority-owned entry and its one-shot reply transport.
use super::super::{BACK, DIR, Fail, Grant, Reply, Wire};
use crate::{
    communication::{
        hand::{Receiver, RecvFail, Sender},
        session::establish,
    },
    wire::message::Message,
};
use env::{PieToken, TaskId, Wait};
use runtime::env::mail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Service(Fail),
    Transport,
    Malformed,
    Discovery,
    WrongAuthority,
    WrongGrant,
}

#[derive(Clone, Copy, Debug)]
pub struct Face {
    authority: TaskId,
    grant: Grant,
    entry: PieToken,
}
impl Face {
    pub fn direct(authority: TaskId, grant: Grant, entry: PieToken) -> Result<Self, CallError> {
        match mail::reserve(entry) {
            Ok((_, owner, mark)) if authority.get() != 0 && owner == authority => {
                if mark != grant.mark() {
                    return Err(CallError::WrongGrant);
                }
                Ok(Self {
                    authority,
                    grant,
                    entry,
                })
            }
            Ok(_) => Err(CallError::WrongAuthority),
            Err(_) => Err(CallError::Transport),
        }
    }
    pub fn discover(
        operator: &crate::system::operator::client::Face,
        authority: TaskId,
        grant: Grant,
        wait: Wait,
    ) -> Result<Self, CallError> {
        let road = DIR.try_join(grant.name()).ok_or(CallError::Discovery)?;
        let tile = operator
            .tile(&road, wait)
            .map_err(|_| CallError::Discovery)?;
        let entry = tile.token(wait).map_err(|_| CallError::Discovery)?;
        Self::direct(authority, grant, entry)
    }
    pub fn authority(&self) -> TaskId {
        self.authority
    }
    pub fn entry(&self) -> PieToken {
        self.entry
    }
    pub fn grant(&self) -> Grant {
        self.grant
    }
    pub fn call(&self, wire: Wire, wait: Wait) -> Result<Reply, CallError> {
        if Grant::for_wire(&wire) != self.grant {
            return Err(CallError::WrongGrant);
        }
        // Revalidate the reference before lending a reply hole.
        Self::direct(self.authority, self.grant, self.entry)?;
        let (back, seed) =
            establish::lend_out(self.entry, BACK).map_err(|_| CallError::Transport)?;
        struct Back(PieToken);
        impl Drop for Back {
            fn drop(&mut self) {
                let _ = mail::seal(self.0);
                let _ = mail::release(self.0);
            }
        }
        let _back = Back(back);
        let mut request = Sender::<(Wire, PieToken)>::from_token(self.entry);
        request
            .send_within((wire, seed), wait)
            .map_err(|_| CallError::Transport)?;
        let mut bytes = Reply::EMPTY;
        let reply = Receiver::<Reply>::from_token(back)
            .recv(&mut bytes, wait)
            .map_err(|error| match error {
                RecvFail::Unread(_) => CallError::Malformed,
                RecvFail::Mail(_) => CallError::Transport,
            })?;
        if let Reply::Fail(fail) = reply {
            return Err(CallError::Service(fail));
        }
        if !reply.belongs_to(self.authority) {
            return Err(CallError::WrongAuthority);
        }
        Ok(reply)
    }
}
impl Reply {
    fn belongs_to(self, authority: TaskId) -> bool {
        match self {
            Self::Binding(Some(b)) => {
                b.origin.principal.authority == authority
                    && b.current.principal.authority == authority
            }
            Self::Principal(Some(p)) => p.authority == authority,
            Self::Coalition(c) => c.authority == authority,
            Self::Members(page) => {
                page.iter().all(|p| p.authority == authority)
                    && page
                        .next()
                        .is_none_or(|c| c.target.authority() == authority)
            }
            Self::Memberships(page) => {
                page.iter().all(|c| c.authority == authority)
                    && page
                        .next()
                        .is_none_or(|c| c.target.authority() == authority)
            }
            _ => true,
        }
    }
}

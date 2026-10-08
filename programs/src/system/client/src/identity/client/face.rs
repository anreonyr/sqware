//! One validated authority-owned entry and its one-shot reply transport.
use super::super::{DIR, Fail, Grant, Reply, Wire, frame::Request};
use ::resource::raw::reserve;
use env::{PieToken, TaskId, Wait};
use ipc::{rpc, time::Deadline};
use system_api::identity::Call;

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
        match reserve(entry) {
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
        operator: &crate::operator::Face,
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
        let deadline = Deadline::new(wait);
        // Revalidate the reference before lending a reply hole.
        Self::direct(self.authority, self.grant, self.entry)?;
        let request = rpc::request::Sender::<Call>::from_raw(self.entry, Call::BACK)
            .map_err(|_| CallError::Transport)?;
        let reply =
            request
                .call(deadline, |back| Request(wire, back))
                .map_err(|error| match error {
                    rpc::Fail::Decode => CallError::Malformed,
                    rpc::Fail::WrongSource => CallError::WrongAuthority,
                    _ => CallError::Transport,
                })?;
        if let Reply::Fail(fail) = reply {
            return Err(CallError::Service(fail));
        }
        if !belongs_to(reply, self.authority) {
            return Err(CallError::WrongAuthority);
        }
        Ok(reply)
    }
}
fn belongs_to(reply: Reply, authority: TaskId) -> bool {
    match reply {
        Reply::Binding(Some(b)) => {
            b.origin.principal.authority == authority && b.current.principal.authority == authority
        }
        Reply::Principal(Some(p)) => p.authority == authority,
        Reply::Coalition(c) => c.authority == authority,
        Reply::Members(page) => {
            page.iter().all(|p| p.authority == authority)
                && page
                    .next()
                    .is_none_or(|c| c.target.authority() == authority)
        }
        Reply::Memberships(page) => {
            page.iter().all(|c| c.authority == authority)
                && page
                    .next()
                    .is_none_or(|c| c.target.authority() == authority)
        }
        _ => true,
    }
}

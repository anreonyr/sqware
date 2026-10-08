//! Pure identity API vocabulary, bounds, marks, and wire representation.

pub mod frame;
pub mod grant;
pub mod limits;
pub mod marks;

pub use frame::vocab::*;
pub use frame::{BACK, Fail, Reply, Request, Wire};
pub use grant::{Grant, Mount, grant_of};
pub const DIR: &str = frame::DIR;
pub use interface::{INTERFACE_ID, REGISTRY};

#[mold::interface(id = "sqware.system.identity.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "back", legacy = "identity-back", constant = BACK)]
        Back,
    }

    #[grants]
    pub enum Grant {
        #[grant(code = 1, key = "resolve", legacy = "identity-resolve")]
        Resolve,
        #[grant(code = 2, key = "matches", legacy = "identity-matches")]
        Matches,
        #[grant(code = 3, key = "same", legacy = "identity-same")]
        Same,
        #[grant(code = 4, key = "sire", legacy = "identity-sire")]
        Sire,
        #[grant(code = 5, key = "heir", legacy = "identity-heir")]
        Heir,
        #[grant(code = 6, key = "amid", legacy = "identity-amid")]
        Amid,
        #[grant(code = 7, key = "members", legacy = "identity-members")]
        Members,
        #[grant(code = 8, key = "memberships", legacy = "identity-memberships")]
        Memberships,
        #[grant(code = 9, key = "adopt", legacy = "identity-adopt")]
        Adopt,
        #[grant(code = 10, key = "waive", legacy = "identity-waive")]
        Waive,
        #[grant(code = 11, key = "restrict", legacy = "identity-restrict")]
        Restrict,
        #[grant(code = 12, key = "derive", legacy = "identity-derive")]
        Derive,
        #[grant(code = 13, key = "found", legacy = "identity-found")]
        Found,
        #[grant(code = 14, key = "admit", legacy = "identity-admit")]
        Admit,
        #[grant(code = 15, key = "expel", legacy = "identity-expel")]
        Expel,
        #[grant(code = 16, key = "bind", legacy = "identity-bind")]
        Bind,
        #[grant(code = 17, key = "unbind", legacy = "identity-unbind")]
        Unbind,
    }
}

fn reply_to(request: &(Option<Wire>, env::PieToken)) -> env::PieToken {
    request.1
}

#[mold::contract(request = Request, response = Reply, mark = interface::BACK, back = reply_to)]
pub struct Call;

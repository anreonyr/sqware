//! Pure Control API types and wire contracts.

pub mod construction;
pub mod frame;
pub mod grant;
pub mod marks;
pub mod observe;

pub use frame::{ASK_MARK, BACK, DENIED, Fail, PublisherAsk, Req, Request, Said, State, Wire};
pub use grant::{Grant, grant_of};
pub use interface::{INTERFACE_ID, REGISTRY};
pub use marks::DECLARATIONS as MARK_DECLARATIONS;
pub const NAME: &str = frame::NAME;
pub const DIR: &crate::operator::path::Path = frame::DIR;
pub const INSTANCE: &crate::operator::Path = crate::operator::Path::new("control/instance");
/// The supervised observation face below the Control service node.
pub const SUPERVISION: &crate::operator::path::Path =
    crate::operator::path::Path::new("control/supervision");
/// The scoped recovery face.
pub const RECOVER: &crate::operator::path::Path =
    crate::operator::path::Path::new("control/recover");

#[mold::interface(id = "sqware.system.control.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "ask", legacy = "control-ask", constant = ASK_MARK)]
        Ask,
        #[channel(key = "back", legacy = "control-back", constant = BACK)]
        Back,
        #[channel(key = "construction", legacy = "control-construction", constant = CONSTRUCTION_ENTRY)]
        ConstructionEntry,
        #[channel(key = "construction-back", legacy = "control-construction-back", constant = CONSTRUCTION_BACK)]
        ConstructionBack,
        #[channel(key = "observe-back", legacy = "control-observe-back", constant = OBSERVE_BACK)]
        ObserveBack,
    }

    #[grants]
    pub enum Grant {
        #[grant(code = 1, key = "state", legacy = "control-entry-state")]
        State,
        #[grant(code = 2, key = "mint", legacy = "control-entry-mint")]
        Mint,
        #[grant(code = 3, key = "embark", legacy = "control-entry-embark")]
        Embark,
        #[grant(code = 4, key = "debark", legacy = "control-entry-debark")]
        Debark,
        #[grant(code = 5, key = "ruin", legacy = "control-entry-ruin")]
        Ruin,
        #[grant(code = 6, key = "observe", legacy = "control-entry-observe")]
        Observe,
        #[grant(code = 7, key = "recover", legacy = "control-entry-recover")]
        Recover,
    }
}

fn reply_to(request: &(Option<Wire>, env::PieToken)) -> env::PieToken {
    request.1
}

#[mold::contract(request = Request, response = Said, mark = interface::BACK, back = reply_to)]
pub struct Call;

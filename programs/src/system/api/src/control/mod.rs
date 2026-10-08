//! Pure Control API types and wire contracts.

pub mod account;
pub mod frame;
pub mod grant;
pub mod marks;
pub mod publication;

pub use frame::{ASK_MARK, BACK, DENIED, Fail, Req, Request, Said, State, Wire};
pub use grant::{Grant, grant_of};
pub use interface::{INTERFACE_ID, REGISTRY};
pub use marks::DECLARATIONS as MARK_DECLARATIONS;
pub use publication::{Frame, Object, Reply, Scope, Target};
pub const NAME: &str = frame::NAME;
pub const DIR: &crate::operator::path::Path = frame::DIR;
pub const INSTANCE: &crate::operator::Path =
    crate::operator::Path::new("/svc/sys/control/instance");

#[mold::interface(id = "sqware.system.control.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "ask", legacy = "control-ask", constant = ASK_MARK)]
        Ask,
        #[channel(key = "back", legacy = "control-back", constant = BACK)]
        Back,
        #[channel(key = "account", legacy = "control-account", constant = ACCOUNT_ENTRY)]
        AccountEntry,
        #[channel(key = "account-back", legacy = "control-account-back", constant = ACCOUNT_BACK)]
        AccountBack,
        #[channel(key = "publication", legacy = "control-publication", constant = PUBLICATION_ENTRY)]
        PublicationEntry,
        #[channel(key = "publication-back", legacy = "control-publication-back", constant = PUBLICATION_BACK)]
        PublicationBack,
        #[channel(key = "identity-ref", legacy = "control-identity-ref", constant = IDENTITY_REF)]
        IdentityRef,
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
    }
}

fn reply_to(request: &(Option<Wire>, env::PieToken)) -> env::PieToken {
    request.1
}

#[mold::contract(request = Request, response = Said, mark = interface::BACK, back = reply_to)]
pub struct Call;

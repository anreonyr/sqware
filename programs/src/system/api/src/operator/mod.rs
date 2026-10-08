//! Pure Operator API: wire types, stable marks, path vocabulary, and bounds.

pub mod frame;
pub mod marks;
pub mod name;
pub mod path;

pub use frame::{
    ASK_MARK, BAD, DENIED, EntryId, Event, FULL, Fail, Kind, Listing, OK, PANE_CAP, Permit, Req,
    Ruling, Said, TIP_BACK, TIP_LEN, TIP_MARK, Tip, TipIn, UNJUDGED, UNKNOWN, Union, Where, Wire,
    code_to_fail, fail_to_code,
};
pub use interface::{INTERFACE_ID, REGISTRY};
pub use marks::{DECLARATIONS as MARK_DECLARATIONS, LINK_MARK, WATCH_MARK};
pub use path::{Path, PathBuf};

pub const NAME: &str = "operator";
pub const SVC: &Path = Path::new("svc");

#[mold::interface(id = "sqware.system.operator.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "ask", legacy = "operator-ask", constant = ASK_MARK)]
        Ask,
        #[channel(key = "tip", legacy = "tip", constant = TIP_MARK)]
        Tip,
        #[channel(key = "tip-back", legacy = "operator-tip-back", constant = TIP_BACK)]
        TipBack,
        #[channel(key = "link", legacy = "operator", constant = LINK_MARK)]
        Link,
        #[channel(key = "watch", legacy = "operator-watch", constant = WATCH_MARK)]
        Watch,
    }
}

#[mold::contract(request = Req, response = Union)]
pub struct Call;

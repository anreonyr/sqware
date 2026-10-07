//! Pure Operator API: wire types, stable marks, path vocabulary, and bounds.

pub mod frame;
pub mod grant;
pub mod marks;
pub mod name;
pub mod path;

pub use frame::{
    ASK_MARK, BAD, DENIED, FULL, LINK, Listing, OK, Req, Said, TIP_BACK, TIP_LEN, TIP_MARK,
    Tip, TipIn, UNJUDGED, UNKNOWN, Union, Wire, code_to_fail, fail_to_code, Permit, Ruling,
    Event, Kind, PANE_CAP, EntryId, Fail, Where,
};
pub use grant::{Grant, grant_of};
pub use marks::{DECLARATIONS as MARK_DECLARATIONS, LINK_MARK, WATCH_MARK};
pub use path::{Path, PathBuf};

pub const NAME: &str = "operator";
pub const SVC: &Path = Path::new("svc");
pub const DIR: &Path = Path::new("svc/sys/operator");
pub const REGISTRY: &[&[env::marks::Definition]] = &[
    &marks::DECLARATIONS,
    &Grant::DECLARATIONS,
];
const _: () = assert!(env::marks::conflict(REGISTRY).is_none());

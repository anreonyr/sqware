#![no_std]

pub mod frame;
pub mod marks;
pub use interface::{INTERFACE_ID, PUBLICATIONS, REGISTRY};

pub use frame::Fail;
pub use marks::{ENTRY_MARK, LANE, LINE_BACK, LINE_MARK};
pub use system_api::operator::path::Path;

pub const SVC: &Path = Path::new("svc");
pub const DIR: &str = "drv";
pub const ROAD: &Path = Path::new("svc/drv");

#[mold::interface(id = "sqware.driver.router.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "entry", legacy = "entry", constant = ENTRY_MARK, publication = "router")]
        Entry,
        #[channel(key = "line-back", legacy = "line-back", constant = LINE_BACK)]
        LineBack,
        #[channel(key = "lane", legacy = "line", constant = LINE_MARK)]
        Line,
    }
}

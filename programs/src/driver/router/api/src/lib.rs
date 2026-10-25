#![no_std]

pub mod declaration;
pub mod frame;
pub mod marks;
pub use declaration::{CHANNEL, E_ROUTER, PLIC_CLASS, READY_MARK};
pub use interface::{INTERFACE_ID, PUBLICATIONS, REGISTRY};

pub use frame::Fail;
pub use marks::{ENTRY_MARK, LANE, LINE_BACK, LINE_MARK};
pub use system_api::operator::path::Path;

pub const ENTRY_NAME: &str = "entry";
pub const ENTRY: &Path = Path::new("router/entry");

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

#![no_std]

extern crate alloc;

pub mod frame;
pub mod grant;
pub mod marks;

pub use frame::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DEV_ROAD, DTB, Deed, ENROLL_CAP,
    ENROLL_MAX, Enroll, Fail, LIST, LIST_MAX, OK, SUPERVISOR_EXTERNAL, Said, TAKEN, UNKNOWN,
    Window, Wire,
};
pub use grant::{Grant, grant_of};
pub use interface::{INTERFACE_ID, PUBLICATIONS, REGISTRY};
pub const NAME: &str = "hub";

#[mold::interface(id = "sqware.service.hub.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "back", legacy = "hub-back", constant = BACK_MARK)]
        Back,
        #[channel(key = "alive", legacy = "hub-alive", constant = ALIVE_MARK)]
        Alive,
    }

    #[grants]
    pub enum Grant {
        #[grant(
            code = 1,
            key = "bond",
            legacy = "hub-entry-bond",
            publication = "bond"
        )]
        Bond,
        #[grant(
            code = 2,
            key = "list",
            legacy = "hub-entry-list",
            publication = "list"
        )]
        List,
        #[grant(
            code = 3,
            key = "claim",
            legacy = "hub-entry-claim",
            publication = "claim"
        )]
        Claim,
    }
}

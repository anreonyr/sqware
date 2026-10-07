#![no_std]

extern crate alloc;

pub mod activation;
pub mod frame;
pub mod grant;
pub mod marks;

pub use frame::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DEV_ROAD, DTB, Deed,
    ENROLL_CAP, ENROLL_MAX, Enroll, Fail, SUPERVISOR_EXTERNAL, LIST, LIST_MAX, OK, TAKEN, UNKNOWN, Window, Said, Wire,
};
pub use grant::{Grant, grant_of};
pub const NAME: &str = "hub";
pub const REGISTRY: &[&[env::marks::Definition]] = &[marks::DECLARATIONS, &Grant::DECLARATIONS];

const _: () = assert!(env::marks::conflict(REGISTRY).is_none(), "hub mark collision");

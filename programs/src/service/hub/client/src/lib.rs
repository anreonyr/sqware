#![no_std]

extern crate alloc;

pub mod client;
mod debug;
pub use client::Face;
pub use hub_api::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DEV_ROAD, DTB, Deed, ENROLL_CAP,
    ENROLL_MAX, Enroll, Fail, Grant, LIST, LIST_MAX, NAME, OK, REGISTRY, SUPERVISOR_EXTERNAL, Said,
    TAKEN, UNKNOWN, Window, Wire, grant_of,
};
pub use hub_api::{activation, frame, grant, marks};

mod activation_client;
pub use activation_client::activate;

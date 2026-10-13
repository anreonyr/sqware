#![no_std]

extern crate alloc;

pub mod client;
mod debug;
pub use client::Face;
pub use hub_api::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DTB, Deed, Fail, Grant,
    INVENTORY_MAX, Inventory, InventoryItem, LIST, LIST_MAX, NAME, OK, REGISTRY, SERVICE_PATH,
    SUPERVISOR_EXTERNAL, Said, TAKEN, UNKNOWN, Window, Wire, grant_of, inventory_slot,
};
pub use hub_api::{frame, grant, marks};

mod activation_client;
pub use activation_client::activate;

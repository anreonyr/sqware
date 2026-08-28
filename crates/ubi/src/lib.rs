#![no_std]
//! U-mode → S-mode 环境调用封装（ubi），独立共享 crate。

pub mod dock;
pub mod fid;
pub mod ucall;

pub use dock::{
    DOCK_KEY_TAG, OFF_BUFFER, OFF_ITEM_LEN, OFF_LOCK, OFF_PIER_COUNT, OFF_QUAY, OFF_READ,
    OFF_SLOTS, OFF_STATE, OFF_WRITE,
};
pub use fid::{ChronoCall, ControlCall, IOCall, MailCall, MemoryCall, RoomCall, TaskCall, Ucall};
pub use ucall::{UArgs, UError, UResult, UcallBuilder};
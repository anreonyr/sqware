#![no_std]
#![feature(allocator_api)]
//! 内核分配器机制：Bump / Frame / Block / Hybrid / Spare 的状态与算法。
//!
//! 这里没有全局实例、没有 `#[global_allocator]`、没有启动或 panic 责任。
//! 每个分配器由使用方给出 [`Lock`] 与 effect；Kernel 与宿主压测台各自保留自己的
//! 实例槽、初始化组合和 effect 实现。

extern crate alloc;

mod error;
mod facts;
mod link;
mod lock;
mod locked;
mod region;

pub(crate) use link::Link;
pub(crate) use locked::Locked;

pub mod block;
pub mod bump;
pub mod frame;
pub mod hybrid;
pub mod spare;

pub use error::{InitError, InitResult};
pub use facts::{MAX_RESERVED, MachineFacts};
pub use lock::{Lock, LockRole};
pub use env::PAGE_SIZE;
pub use region::Region;

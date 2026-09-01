#![no_std]
//! 用户态系统调用封装（U-mode → S-mode 环境调用）。

extern crate alloc;

pub mod entry;
pub mod core;
pub mod lisp;
pub mod env;
pub(crate) mod shared;

pub const PAGE_SIZE: usize = 4096;
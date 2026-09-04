#![no_std]
//! 用户态系统调用封装（U-mode → S-mode 环境调用）。

extern crate alloc;

pub mod core;
pub mod entry;
pub mod env;
pub mod lisp;

pub const PAGE_SIZE: usize = 4096;

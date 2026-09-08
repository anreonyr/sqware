#![no_std]
//! 镜像侧环境调用封装（U 态程序与 S 态域程序共用；特权级由内核装载时决定）。

extern crate alloc;

pub mod core;
pub mod entry;
pub mod env;
pub mod lisp;
pub mod term;

pub const PAGE_SIZE: usize = 4096;

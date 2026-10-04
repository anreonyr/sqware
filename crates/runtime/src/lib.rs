#![no_std]
//! 镜像侧运行时：资源操作、任务本地原语与内存管理。
//! 共享 ABI 与调用入口由 `env` 提供。

extern crate alloc;

pub mod core;

pub const PAGE_SIZE: usize = 4096;

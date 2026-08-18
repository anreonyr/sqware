#![no_std]
//! U-mode → S-mode 环境调用封装（ubi），独立共享 crate。
//!
//! 镜像内核侧 S-mode → M-mode 的 `sbi` crate：调用号契约（`fid::Ucall`）与
//! 调用构建器/错误（`ucall::UcallBuilder`、`UError`、`UResult`）同构；`warpper`
//! 是唯一碰汇编的原语。kernel 与 user 共同依赖，杜绝调用号双份漂移。

pub mod fid;
pub mod ucall;

pub use fid::Ucall;
pub use ucall::{UArgs, UError, UResult, UcallBuilder};

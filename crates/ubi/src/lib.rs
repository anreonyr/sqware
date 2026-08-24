#![no_std]
//! U-mode → S-mode 环境调用封装（ubi），独立共享 crate。

pub mod fid;
pub mod ucall;

pub use fid::Ucall;
pub use ucall::{UArgs, UError, UResult, UcallBuilder};

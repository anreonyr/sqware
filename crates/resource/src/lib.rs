#![no_std]
//! 用户态资源封装：孔通信、页映射、门铃与多路等待。
//!
//! 原始能力表查询与 token 适配集中在 [`raw`]，不表示资源已验证。持有与派生授予分别
//! 管理释放和撤销；正式使用面是 `dock`、`port`、`bell` 与 `pile`。

pub mod bell;
mod capability;
pub mod dock;
mod hole;
pub mod pile;
pub mod port;
pub mod raw;
mod reply;

pub use env::PAGE_SIZE;

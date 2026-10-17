#![no_std]
//! 用户态资源封装：孔通信、页映射、门铃与多路等待。
//!
//! 原始能力表查询与 token 适配集中在 [`raw`]，不表示资源已验证。持有与派生授予分别
//! 管理释放和撤销；正式使用面是 `dock`、`port`、`bell` 与 `pile`。
//! [`retention`] 是跨任务的引用生命周期核心：引用活到受信任的停机权威确认外部访问者停下为止。

extern crate alloc;

pub mod bell;
mod capability;
pub mod dock;
mod hole;
pub mod pile;
pub mod port;
pub mod raw;
mod reply;
pub mod retention;

pub use env::PAGE_SIZE;

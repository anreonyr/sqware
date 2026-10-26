#![no_std]
//! 用户态资源封装：孔通信、页映射、门铃与多路等待。
//!
//! 原始能力表查询与 token 适配集中在 [`raw`]，不表示资源已验证。持有与派生授予分别
//! 管理释放和撤销；正式使用面是 `dock`、`port`、`bell` 与 `pile`。
//!
//! 能力与映射各自经静态 [`kernel`] / [`mapping_kernel`] 缝调用内核；默认后端是
//! [`native::Native`]，测试夹具可给出自己的单次调用适配，而 `Capability`、`Loan` 与
//! `Dock` 仍走同一份真实机制。

pub mod bell;
pub mod capability;
pub mod cleanup;
pub mod dock;
mod hole;
pub mod kernel;
pub mod loan;
pub mod mapping;
pub mod mapping_kernel;
pub mod native;
pub mod pile;
pub mod port;
mod port_open_fail;
pub mod raw;
mod reply;
mod reply_error;
pub mod view;

pub use env::PAGE_SIZE;

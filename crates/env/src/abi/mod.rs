//! abi — **环境调用那一面**：调用号与载荷 codec（[`call`]）· 期限（[`wait`]）· 出口码（[`exit`]）·
//! 门闩权限（[`permission`]）。**发起骨架 [`crate::ecall`] 留在 crate 根**：`mold` 的
//! `#[derive(Envcall)]` 展开体写死 `crate::ecall::{make_fail,trap}`（宏绑定路径，与
//! `programs::entry` 同一条道理）。

pub mod call;
pub mod exit;
pub mod permission;
pub mod wait;

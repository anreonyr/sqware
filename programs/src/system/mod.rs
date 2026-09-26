//! system — **编排域的实现侧**：起一条、看/判、放下、收场。
//!
//! 系统协议的规范与接口住在 `crates/protocol/src/system/`；本目录是装配、监督、与收场的实现。
//!
//! - [`assemble`]：这一景起哪些程序、装配参数
//! - [`board`] / [`operator`] / [`principal`] / [`coalition`]：四枚本域常驻线程的实现
//! - [`machine`]：本域手里那台机器的自述（设备树）
//! - [`server`]：装配期的原语（建域、放行、等就绪、收）
//! - [`supervise`]：监督相（名册还剩几位、最后一位走了怎么收）

pub mod assemble;
pub mod board;
pub mod coalition;
pub mod machine;
pub mod operator;
pub mod principal;
pub mod server;
pub mod supervise;

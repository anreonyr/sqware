#![no_std]
#![no_main]

//! coalition — **盟册（结盟服务）**（U 态，**独立域**）：答"这一位在那枚盟里吗"。
//!
//! 起手一把在 [`server::serve`]：上板 → 铸门牌 → 上树 → 找身份那一份 → 空册 → 常驻那一问。
//!
//! **它与其他每一台走同一条路**：编排域按装配表（`programs::program::PROGRAMS`，order 2）用 `mint`
//! 建这个域、产这一枚线程；它认"起我那一枚线程"只有一条 —— `runtime::env::unit::sire()`。
//! iii 那套"同域产线程、拿 `args` 补一格"的特例连同 `Role` 一起退了。

extern crate programs;

use programs::system::control::service as core;
use programs::system::coalition;

#[programs::entry]
fn main() -> Result<(), core::Start> {
    coalition::server::serve()
}

#![no_std]
#![no_main]

//! principal — **名册（身份服务）**（U 态，**独立域**）：装配期每一条服务的号都从它来。
//!
//! 起手一把在 [`server::serve`]：上板 → 铸门牌 → 上树 → 两张表（谱系 + 名册）→ 常驻那一问。
//!
//! **它与其他每一台走同一条路**：编排域按装配表（`programs::unit::PROGRAMS`，order 1）用 `mint`
//! 建这个域、产这一枚线程；它认"起我那一枚线程"只有一条 —— `runtime::env::unit::sire()`。
//! iii 那套"同域产线程、拿 `args` 补一格"的特例连同 `Role` 一起退了。

extern crate programs;

use programs::system::control::service as core;
use programs::service::principal;

#[programs::entry]
fn main() -> Result<(), core::Start> {
    principal::server::serve()
}

#![no_std]
#![no_main]

//! operator — **持树者**（U 态，**独立域**）：客人上树要它在。
//!
//! 起手一把在 [`server::serve`]：上板 → 铸提示孔交给装配者 → 一枚线程招待所有客人。
//!
//! **它与其他每一台走同一条路**：编排域按装配表（`programs::unit::PROGRAMS`，order 0）用 `mint`
//! 建这个域、产这一枚线程；它认"起我那一枚线程"只有一条 —— `runtime::env::unit::sire()`
//! （建这个域的就是装配者）。iii 那套"与编排者共一份字节、同域产线程、拿 `args` 补一格"的
//! 特例连同 `Role` 一起退了。

extern crate programs;

use programs::system::control::service as core;
use programs::service::operator;

#[programs::entry]
fn main() -> Result<(), core::Start> {
    operator::server::serve()
}

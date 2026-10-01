#![no_std]
#![no_main]

//! hub — **设备账（设备 hub）**（U 态，**独立域**）：回答"这台机器上有哪些设备、谁在驱它们"。
//!
//! 起手一把在 [`server::serve`]：收整机物料 → 立账 → 上树 → 逐类立盟 → 落 `/svc/hub` 与 `/dev`
//! → 一枚线程招待所有客人（三面：报名 / 列册 / 认领）。
//!
//! **它与其他每一台走同一条路**：编排域按装配表（`programs::unit::PROGRAMS`，order 3）用
//! `mint` 建这个域、产这一枚线程；它认"起我那一枚线程"只有一条 —— `runtime::env::unit::sire()`。
//! **它的整机物料从装配者那条通道来**（`Setup::Machine`，见 `programs/src/system/hub/program.rs`）。

extern crate programs;

use programs::system::control::service as core;
use programs::service::hub;

#[programs::entry]
fn main() -> Result<(), core::Start> {
    hub::server::serve()
}

#![no_std]
#![no_main]

//! 回答"这台机器上有哪些设备、谁在驱它们"。
//! 起手一把在 server::serve：收整机物料 → 立账 → 上树 → 逐类立盟 → 落 `/svc/hub` 与 `/dev`
//! → 一枚线程招待所有客人（三面：报名 / 列册 / 认领）。
//! **它与其他每一台走同一条路**：编排域按装配表（programs::unit::PROGRAMS，order 3）用
//! `mint` 建这个域、产这一枚线程；它认"起我那一枚线程"只有一条 —— `env::unit::sire()`。

extern crate programs;

use programs::service::hub;

#[programs::entry]
fn main() -> Result<(), hub::Start> {
    hub::serve::serve()
}

#![no_std]
#![no_main]

//! 外部中断的收与结（U 态，一枚线程）。
//! （`adapt/{sweep,resident}.rs ＋ adapt/event/{bell,desk,exhaust}.rs`），那一圈的**壳**在 `adapt/resident.rs`；
//! 树那侧的事实与线集合在 `core/sources.rs`（纯），寄存器面在 `dev/plic.rs`（设备）。
//! 判据、裁法与那一张读数表在 `driver/router/mod.rs`。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

/// 住持面（适配）：起手 / 门面 / 逐客 / 排空 / 铃 / 常驻——由 bin 自己 `mod`
mod adapt;

/// 纯功能：树那侧的事实与线集合（区 ↔ 线号）
mod core;

mod dev;

#[programs::entry]
fn main() -> Result<(), programs::driver::shared::fail::Fail> {
    // 起手：领配给 → 开两图 → 读树 → 建账 → 铸入口 → 上板 ＋ 上树 → 挂组。
    let mut up = adapt::boot::up()?;
    // 常驻：等三源 → 逐客 / 排空 / 登记 / 铃。
    adapt::resident::run(&mut up)
}

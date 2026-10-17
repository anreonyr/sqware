#![no_std]
#![no_main]

//! 本景的控制单元（身子在 `..` 的 `unit`）。
//!
//! 它由引导那一台装载成**一台真任务**：有 sire，故与它同 Team 的故障任务都带那条
//! Team 的收据——按页错误真终止的任务因此不会被当成无人认领的塌方。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::memory_transfer::unit();
    programs::Report::note(env::EXIT_OK, "memory-transfer: run passed")
}

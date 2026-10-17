#![no_std]
#![no_main]

//! 本景的引导那一台：装载控制单元并把控制台门闩授过去（身子在 `..` 的 `acceptance`）。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::memory_transfer::acceptance();
    programs::Report::note(env::EXIT_OK, "memory-transfer: acceptance passed")
}

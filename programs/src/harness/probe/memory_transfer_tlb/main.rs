#![no_std]
#![no_main]

//! 本景的引导那一台：它自己就是控制者（身子在 `..` 的 `acceptance`）。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::memory_transfer_tlb::acceptance();
    programs::Report::note(env::EXIT_OK, "memory-transfer-tlb: acceptance passed")
}

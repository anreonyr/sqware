#![no_std]
#![no_main]

//! 本景的**对端**那一台（身子在 `..` 的 `peer`）：它由管理端装载进另一个 Team，
//! 故它那一域有自己的 Space，而那一域的收据归装载者。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::memory_transfer_race::run();
    programs::Report::note(env::EXIT_OK, "memory-transfer-race: peer passed")
}

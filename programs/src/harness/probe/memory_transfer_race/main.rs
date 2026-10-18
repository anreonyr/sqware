#![no_std]
#![no_main]

//! 真实多 Hart 交错那一景的引导镜像（**管理端**）：身子在 `..` 的 `acceptance`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::memory_transfer_race::acceptance();
    programs::Report::note(env::EXIT_OK, "memory-transfer-race: acceptance passed")
}

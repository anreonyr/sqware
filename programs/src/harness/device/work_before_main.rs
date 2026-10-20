#![no_std]
#![no_main]

//! α（发布前中止那一档）的引导镜像：身子在 `..` 的 `work::run`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::work::run(programs::harness::device::work::Mode::Before) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-work: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

#![no_std]
#![no_main]

//! 控制器（正常那一档）的引导镜像：身子在 `..` 的 `control::run`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Normal) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

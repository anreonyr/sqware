#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Guard) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl-guard: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Soak) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl-soak: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

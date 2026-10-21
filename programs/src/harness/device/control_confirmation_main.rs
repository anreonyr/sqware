#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Confirmation) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl-confirmation: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Forced) {
        Ok(()) => programs::Report::note(env::EXIT_FAULT, "device-ctl-forced: unexpected return"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

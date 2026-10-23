#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_target::controller() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "daemon-ctl-target: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

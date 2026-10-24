#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_target::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, programs::harness::probe::daemon_target::done()),
        Err(why) => programs::Report::note(why, "daemon-target: failed"),
    }
}

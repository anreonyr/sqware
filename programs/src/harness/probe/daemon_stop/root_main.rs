#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_stop::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, programs::harness::probe::daemon_stop::done()),
        Err(why) => programs::Report::note(why, "daemon-stop: failed"),
    }
}

#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_stop::controller() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "daemon-ctl-stop: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

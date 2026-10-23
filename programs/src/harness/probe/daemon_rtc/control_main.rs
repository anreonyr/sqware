#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_rtc::controller() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "daemon-ctl-rtc: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

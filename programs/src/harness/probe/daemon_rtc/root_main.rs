#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::daemon_rtc::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "daemon-rtc: done"),
        Err(why) => programs::Report::note(why, "daemon-rtc: failed"),
    }
}

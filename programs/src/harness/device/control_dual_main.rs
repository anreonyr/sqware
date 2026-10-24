#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    let generations = match execution::boot::args::args() {
        [] => 1,
        [1024] => 1024,
        _ => return programs::Report::note(env::EXIT_FAULT, "device-ctl-dual: invalid generations"),
    };
    match programs::harness::device::dual::run(generations) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl-dual: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

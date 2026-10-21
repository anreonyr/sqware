#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    let generations = match execution::boot::args::args() {
        [] => 128,
        [count] if matches!(*count, 128 | 1024) => *count,
        _ => {
            return programs::Report::note(
                env::EXIT_FAULT,
                "device-ctl-soak: invalid generation count",
            );
        }
    };
    match programs::harness::device::control::run(programs::harness::device::control::Mode::Soak {
        generations,
    }) {
        Ok(()) => programs::Report::note(env::EXIT_OK, "device-ctl-soak: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

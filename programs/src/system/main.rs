#![no_std]
#![no_main]
extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::system::control::serve::run::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "system: done"),
        Err(reason) => programs::Report::note(reason, "system: failed"),
    }
}

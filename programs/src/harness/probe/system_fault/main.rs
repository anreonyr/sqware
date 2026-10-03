#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::lifecycle::acceptance();
    programs::harness::probe::copy::acceptance();
    programs::harness::probe::identity::acceptance();
    programs::harness::probe::system_fault::acceptance();
    programs::Report::note(env::EXIT_OK, "system-fault: acceptance passed")
}

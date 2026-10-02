#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::identity::replacement();
    programs::Report::note(env::EXIT_OK, "identity-replacement: IPC acceptance passed")
}

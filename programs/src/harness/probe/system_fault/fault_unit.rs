#![no_std]
#![no_main]
extern crate programs;
#[programs::entry]
fn main() -> env::Reason {
    programs::harness::probe::system_fault::unit();
    env::EXIT_OK
}

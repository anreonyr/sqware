#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Reason {
    programs::harness::bench::load::run()
}

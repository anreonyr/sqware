#![no_std]
#![no_main]

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::acceptance()
}

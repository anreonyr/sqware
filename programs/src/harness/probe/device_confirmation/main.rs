#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::device_confirmation::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "iommu-confirmation: done"),
        Err(why) => programs::Report::note(why, "iommu-confirmation: failed"),
    }
}

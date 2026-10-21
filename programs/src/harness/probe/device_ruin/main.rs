#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::device_ruin::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "iommu-ruin: done"),
        Err(why) => programs::Report::note(why, "iommu-ruin: failed"),
    }
}

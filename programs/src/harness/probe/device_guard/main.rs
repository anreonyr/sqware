#![no_std]
#![no_main]

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::harness::probe::device_guard::run() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "iommu-guard: done"),
        Err(why) => programs::Report::note(why, "iommu-guard: failed"),
    }
}

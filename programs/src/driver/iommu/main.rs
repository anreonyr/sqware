#![no_std]
#![no_main]

//! IOMMU 驱动那一台的引导镜像（生产只此一张）：身子在 `..` 的 `serve`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::driver::iommu::serve::serve() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "iommu: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

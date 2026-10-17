#![no_std]
#![no_main]

//! 本景的引导镜像：它自己就是控制者（身子在 `..` 的 `dma::acceptance`）。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    programs::harness::probe::iommu::dma::acceptance();
    programs::Report::note(env::EXIT_OK, "iommu-dma: acceptance passed")
}

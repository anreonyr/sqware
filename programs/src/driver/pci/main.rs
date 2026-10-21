#![no_std]
#![no_main]

//! PCI 驱动那一台的引导镜像：身子在 `..` 的 `serve`。

extern crate programs;

#[programs::entry]
fn main() -> programs::Report<'static> {
    match programs::driver::pci::serve::serve() {
        Ok(()) => programs::Report::note(env::EXIT_OK, "pci: done"),
        Err(why) => programs::Report::note(env::EXIT_FAULT, why),
    }
}

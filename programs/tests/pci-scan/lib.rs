#![cfg(test)]

#[path = "../../src/driver/pci/end.rs"]
pub mod end;
#[path = "../../src/driver/pci/function.rs"]
pub mod function;
#[path = "../../src/driver/pci/page.rs"]
pub mod page;
#[path = "../../src/driver/pci/reader.rs"]
pub mod reader;
#[path = "../../src/driver/pci/scan.rs"]
pub mod scan;
#[path = "../../src/driver/pci/snapshot.rs"]
pub mod snapshot;
#[path = "../../src/driver/pci/window.rs"]
pub mod window;

#[path = "fixtures/sparse.rs"]
pub mod sparse;
#[path = "fixtures/memory.rs"]
pub mod memory;

mod tests;

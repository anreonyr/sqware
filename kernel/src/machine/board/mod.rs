mod error;
mod machine;
mod region;

pub use error::MachineError;
pub use machine::{Machine, MAX_RESERVED};
pub use region::Region;

use crate::machine::hart;
use crate::machine::sync::OnceLock;

static MACHINE: OnceLock<Machine> = OnceLock::new();

pub fn init(dtp: usize) -> Result<(), MachineError> {
    let machine = Machine::parse(dtp, hart::hart_id())?;
    assert!(MACHINE.set(machine).is_ok(), "machine already initialized");
    Ok(())
}

pub fn info() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
}

pub fn device_tree() -> Option<fdt::Fdt<'static>> {
    let dtb = MACHINE.get()?.dtb();
    unsafe { fdt::Fdt::from_ptr(dtb.base as *const u8) }.ok()
}

pub(crate) fn dram_edge() -> Option<usize> {
    MACHINE.get().map(|m| m.dram.range().end)
}

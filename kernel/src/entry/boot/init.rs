use super::{BootError, ResourceOperation};
use crate::core::memory::{allocator, manager};
use crate::core::work::{record::trace, room::interrupt};
use crate::entry::call;
use crate::machine::{board, clock, console};

pub fn init(dtp: usize) -> Result<(), BootError> {
    console::init();
    board::init(dtp).map_err(|source| BootError::Machine { dtp, source })?;
    allocator::init().map_err(BootError::Allocator)?;
    manager::mode::detect().map_err(BootError::PagingMode)?;
    super::unit::init().map_err(BootError::KernelSpace)?;
    clock::init().map_err(BootError::Clock)?;
    #[cfg(not(feature = "semihosting"))]
    trace::init().map_err(BootError::Trace)?;
    #[cfg(feature = "semihosting")]
    trace::init(crate::entry::report::export::note_sink).map_err(BootError::Trace)?;
    interrupt::init().map_err(|source| BootError::Resources {
        operation: ResourceOperation::InitializeTraps,
        source,
    })?;
    call::resources::init().map_err(|source| BootError::Resources {
        operation: ResourceOperation::InitializeCalls,
        source,
    })?;
    super::stack::init().map_err(BootError::TrapStacks)?;
    Ok(())
}

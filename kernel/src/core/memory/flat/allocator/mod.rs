pub mod bitmap;
pub mod block;
pub mod bump;
pub mod frame;
pub mod hybrid;
mod no_allocation;
pub mod portal;
pub mod spare;
pub mod statistics;

pub use kernel_allocator::{InitError, InitResult};
#[cfg(debug_assertions)]
pub(crate) use no_allocation::NoAllocation;
pub(crate) use no_allocation::assert_allocation_allowed;

pub fn init() -> InitResult<()> {
    bump::init()?;
    portal::switch(portal::Backend::Bump);

    statistics::init().map_err(|error| {
        erra::Error::new(
            "initializing allocator statistics",
            match error {
                statistics::Error::AlreadyInitialized => InitError::AlreadyInitialized,
                statistics::Error::OutOfMemory => InitError::OutOfMemory,
            },
        )
    })?;

    hybrid::init()?;
    portal::switch(portal::Backend::Hybrid);
    spare::init()?;

    statistics::record_spare_total(spare::spare().total_bytes());

    Ok(())
}

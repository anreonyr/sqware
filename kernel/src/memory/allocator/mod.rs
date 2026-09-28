use core::ptr::NonNull;

use fack::prelude::Error;

pub mod bitmap;
pub mod block;
pub mod bump;
pub mod frame;
pub mod hybrid;
pub mod portal;
pub mod spare;
pub mod statistics;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(unused)]
pub enum InitError {
    #[error("no free memory region configured")]
    NoFreeMemory,
    #[error("memory allocation failed while initializing allocator")]
    OutOfMemory,
    #[error("no free physical frames available")]
    NoFreeFrames,
    #[error("no harts reported")]
    NoHarts,
    #[error("allocator already initialized")]
    AlreadyInitialized,
}

pub type InitResult<T> = erra::Result<T, InitError>;

struct Link {
    prev: Option<NonNull<Link>>,
    next: Option<NonNull<Link>>,
}

impl Link {
    fn new(prev: Option<NonNull<Link>>, next: Option<NonNull<Link>>) -> Self {
        Self { prev, next }
    }
}

pub fn init() -> InitResult<()> {
    bump::init()?;
    portal::switch(portal::Backend::Bump);

    statistics::init().expect("statistics init: already initialized");

    hybrid::init()?;
    portal::switch(portal::Backend::Hybrid);
    spare::init()?;

    statistics::record_spare_total(spare::spare().total_bytes());

    Ok(())
}
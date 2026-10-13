mod error;
mod node;
mod page;

pub use error::MapError;
pub(crate) use node::TableNode;
pub(crate) use page::PageTable;

#[cfg(debug_assertions)]
pub(crate) use node::large_pages_accept;

pub(crate) static TABLE_LIVE: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

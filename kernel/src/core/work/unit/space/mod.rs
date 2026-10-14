mod builder;
mod mapping;
mod memory;
mod space;

#[cfg(debug_assertions)]
pub use crate::core::memory::nest::SpaceKind;
pub(crate) use mapping::Mapping;
pub(crate) use memory::Memory;
pub use builder::SpaceBuilder;
pub use space::Space;

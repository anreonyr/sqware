mod builder;
mod space;

#[cfg(debug_assertions)]
pub use crate::core::memory::nest::SpaceKind;
pub use builder::SpaceBuilder;
pub use space::Space;

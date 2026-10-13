mod builder;
mod space;

#[cfg(debug_assertions)]
pub use crate::core::memory::manager::SpaceKind;
pub use builder::SpaceBuilder;
pub use space::Space;

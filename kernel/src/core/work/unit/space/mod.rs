mod builder;
mod space;
mod view_entry;
mod views;

#[cfg(debug_assertions)]
pub use crate::core::memory::nest::SpaceKind;
pub use builder::SpaceBuilder;
pub use space::Space;
pub(crate) use view_entry::ViewEntry;
pub(crate) use views::ViewRegistry;

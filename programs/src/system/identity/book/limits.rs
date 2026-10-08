//! Authority storage limits.

pub use system_api::identity::limits::PAGE_ITEMS;
pub const MAX_PRINCIPALS: usize = 4096;
pub const MAX_COALITIONS: usize = 4096;
pub const MAX_MEMBERSHIPS: usize = 16384;
pub const MAX_BINDINGS: usize = 4096;
pub const MAX_CREATED_PER_PRINCIPAL: usize = 256;

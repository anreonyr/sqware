//! Shared resource boundaries. Overflow fails; it never truncates.
pub const MAX_ACTIVE_COALITIONS: usize = 16;
pub const MAX_PAGE_ITEMS: usize = 16;
pub const PAGE_ITEMS: usize = MAX_PAGE_ITEMS;
pub const MAX_FRAME: usize = 1024;
pub const MAX_PRINCIPALS: usize = 4096;
pub const MAX_COALITIONS: usize = 4096;
pub const MAX_MEMBERSHIPS: usize = 16384;
pub const MAX_BINDINGS: usize = 4096;
pub const MAX_CREATED_PER_PRINCIPAL: usize = 256;

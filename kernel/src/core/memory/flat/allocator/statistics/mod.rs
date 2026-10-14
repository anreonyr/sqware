mod error;
mod stats;

#[cfg(debug_assertions)]
#[path = "block_kinds.rs"]
mod block_kinds;
#[cfg(debug_assertions)]
#[path = "frame_kinds.rs"]
mod frame_kinds;
#[cfg(debug_assertions)]
#[path = "kind.rs"]
mod kind;
#[cfg(debug_assertions)]
#[path = "kinds.rs"]
mod kinds;
#[cfg(debug_assertions)]
#[path = "mark.rs"]
mod mark;
#[cfg(debug_assertions)]
#[path = "mark_cell.rs"]
mod mark_cell;

pub use error::Error;
pub(crate) use stats::{
    init, record_block_give, record_block_take, record_frame_give, record_frame_take,
    record_pool_give, record_pool_take, record_spare_give, record_spare_take, record_spare_total,
};

#[cfg(debug_assertions)]
pub(crate) use block_kinds::install_block_kinds;
#[cfg(debug_assertions)]
pub(crate) use frame_kinds::install_frame_kinds;
#[cfg(debug_assertions)]
pub(crate) use kind::Kind;
#[cfg(debug_assertions)]
pub(crate) use mark::mark;
#[cfg(debug_assertions)]
pub(crate) use stats::kinds;
#[cfg(debug_assertions)]
pub use stats::{block_occupied, frame_occupied, spare_available, spare_occupied};

#[cfg(not(debug_assertions))]
pub(crate) fn install_frame_kinds(_frames: usize) -> Result<(), super::InitError> {
    Ok(())
}

#[cfg(not(debug_assertions))]
pub(crate) fn install_block_kinds(_base: usize, _len: usize) -> Result<(), super::InitError> {
    Ok(())
}

#[cfg(debug_assertions)]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {{
        let _m = $crate::core::memory::flat::allocator::statistics::mark(
            $crate::core::memory::flat::allocator::statistics::Kind::$kind,
        );
        $v
    }};
}

#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {
        $v
    };
}

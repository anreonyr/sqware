pub mod hole;
pub mod nole;
pub mod pole;
pub mod tole;

pub(crate) use hole::HoleMeta;
pub(crate) use pole::PoleMeta;
pub(crate) use tole::ToleMeta;

use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;

pub(crate) fn whole(space: &Space, va: usize, len: usize, need: PteFlags) -> bool {
    space.validate_access(va, len, need)
}

pub(crate) fn copy_in(space: &Space, dst: &mut [u8], va: usize) -> bool {
    space.copy_in(dst, va)
}

pub(crate) fn copy_out(space: &Space, src: &[u8], va: usize) -> bool {
    space.copy_out(src, va)
}

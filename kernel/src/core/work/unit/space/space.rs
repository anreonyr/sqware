use alloc::sync::{Arc, Weak};

use crate::core::memory::address::VirtAddr;
use crate::core::memory::nest::{Access, Nest, SpaceKind};
use crate::core::work::unit::life::Life;

use super::ViewRegistry;

/// 一个团队的地址空间：CPU 翻译（`Nest`）加一条生命周期线。
///
/// 来源准备与选址组合由调用方显式给出；这里只保留“本空间的用户半边”这一条
/// 访问纪律：地址窗口与页表权限同时成立才放行。
/// `views` 是新 ABI 的**控制授权记录**，不是页槽副本。
pub struct Space {
    pub(crate) nest: Arc<Nest>,
    pub(super) life: Arc<Life>,
    pub(crate) views: ViewRegistry,
}

impl Space {
    fn release_controller(id: usize, entry: &super::ViewEntry) {
        if entry.scope == env::ViewControl::Caller
            && let Some(task) = crate::core::work::room::scheduler::roster::muster(entry.controller)
                .and_then(|task| task.upgrade())
        {
            task.release_control(id);
        }
    }

    pub(crate) fn retire_view(&self, id: usize) {
        if let Some(entry) = self.views.locate(id) {
            self.views.retire(id);
            Self::release_controller(id, &entry);
        }
    }

    pub(crate) fn reap_views(&self) {
        while let Some((id, entry)) = self.views.take_settled(&self.nest) {
            Self::release_controller(id, &entry);
        }
    }

    pub fn kind(&self) -> SpaceKind {
        self.nest.kind()
    }

    pub fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    /// 一段字节窗口是否整段落在用户可达的半边之内。
    pub(crate) fn user_range(at: VirtAddr, size: usize) -> bool {
        let ceiling = crate::machine::mode::upper().as_usize();
        at.as_usize() < ceiling
            && at
                .as_usize()
                .checked_add(size)
                .is_some_and(|end| end <= ceiling)
    }

    /// 用户半边的一次访问判定。
    pub(crate) fn validate_access(&self, at: VirtAddr, len: usize, need: Access) -> bool {
        Self::user_range(at, len) && self.nest.check_access(at, len, need)
    }

    pub(crate) fn copy_in(&self, dst: &mut [u8], at: VirtAddr) -> bool {
        Self::user_range(at, dst.len()) && self.nest.copy_from(at, dst, Access::READ)
    }

    pub(crate) fn copy_out(&self, src: &[u8], at: VirtAddr) -> bool {
        Self::user_range(at, src.len()) && self.nest.copy_to(at, src, Access::WRITE)
    }

    /// 取一段指令的首字节：取不到就没有可取的长度。
    pub(crate) fn instruction_byte(&self, at: VirtAddr) -> Option<u8> {
        let mut byte = [0];
        (Self::user_range(at, byte.len()) && self.nest.copy_from(at, &mut byte, Access::EXECUTE))
            .then_some(byte[0])
    }
}

unsafe impl Send for Space {}
unsafe impl Sync for Space {}

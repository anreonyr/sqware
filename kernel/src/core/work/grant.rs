use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use env::Permission;

use crate::core::memory::address::{PageCount, PageOffset};

pub(crate) fn form_ok(src: Permission, subset: Permission) -> bool {
    src.contains(Permission::ONLY) == subset.contains(Permission::ONLY)
}

pub(crate) fn narrows(from: Permission, to: Permission) -> bool {
    !to.is_empty() && from.contains(to) && form_ok(from, to)
}

pub(crate) struct Grant {
    permission: AtomicU32,
    /// 这一枚分支的有效窗口：**相对来源起点的绝对页偏移与页数**（不是相对父分支的偏移，
    /// 故嵌套派生不会叠加两层偏移）。`None` = 整段来源（旧行为）。
    /// 只在建这一枚时收窄一次，之后没有放宽的路。
    window: Option<(PageOffset, PageCount)>,
}

impl Grant {
    pub(crate) fn try_new(permission: Permission) -> Result<Arc<Self>, env::PieFail> {
        Self::try_new_window(permission, None)
    }

    pub(crate) fn try_new_window(
        permission: Permission,
        window: Option<(PageOffset, PageCount)>,
    ) -> Result<Arc<Self>, env::PieFail> {
        Arc::try_new(Self {
            permission: AtomicU32::new(permission.bits()),
            window,
        })
        .map_err(|_| env::PieFail::OoM)
    }

    /// 这一枚分支的有效窗口；`None` = 整段来源。
    pub(crate) fn window(&self) -> Option<(PageOffset, PageCount)> {
        self.window
    }

    pub(crate) fn permission(&self) -> Option<Permission> {
        let bits = self.permission.load(Ordering::Acquire);
        (bits != 0).then(|| Permission::from_bits_retain(bits))
    }

    pub(crate) fn narrow(&self, subset: Permission) -> Result<(), env::PieFail> {
        let subset_bits = subset.bits();
        let mut current = self.permission.load(Ordering::Acquire);
        loop {
            if current == 0 {
                return Err(env::PieFail::Dead);
            }
            let permission = Permission::from_bits_retain(current);
            if !narrows(permission, subset) {
                return Err(env::PieFail::Denied);
            }
            match self.permission.compare_exchange_weak(
                current,
                current & subset_bits,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn invalidate(&self) {
        self.permission.store(0, Ordering::Release);
    }
}

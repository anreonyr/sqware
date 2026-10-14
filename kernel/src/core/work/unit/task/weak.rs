use alloc::sync::{Arc, Weak};

#[cfg(debug_assertions)]
use crate::core::work::unit::copies;
use crate::core::work::unit::copies::Origin;

use super::Task;

/// 一个弱抄件：团队成员表、名册、等待表、家长位只挂它，不给任务强引用。调试档下
/// 每个抄件还登记一个槽位，供「跨挂起仍有抄件」自检。
pub(crate) struct TaskWeak {
    w: Weak<Task>,
    #[cfg(debug_assertions)]
    slot: usize,
}

impl TaskWeak {
    pub(crate) fn stored(w: Weak<Task>, origin: Origin) -> TaskWeak {
        #[cfg(not(debug_assertions))]
        let _ = origin;
        TaskWeak {
            w,
            #[cfg(debug_assertions)]
            slot: copies::register(origin),
        }
    }

    pub(crate) fn live(&self) -> bool {
        self.w.strong_count() > 0
    }

    pub(crate) fn as_weak(&self) -> &Weak<Task> {
        &self.w
    }

    pub(crate) fn upgrade(&self) -> Option<Arc<Task>> {
        self.w.upgrade()
    }

    pub(crate) fn copy_at(&self, origin: Origin) -> TaskWeak {
        TaskWeak::stored(self.w.clone(), origin)
    }

    pub(crate) fn empty() -> TaskWeak {
        TaskWeak {
            w: Weak::new(),
            #[cfg(debug_assertions)]
            slot: 0,
        }
    }
}

impl Drop for TaskWeak {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        copies::release(self.slot);
    }
}

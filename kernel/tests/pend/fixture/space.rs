/// 宿主两侧共用的真实绑定：它只依赖 `Weak<Space>` 与一把短锁。
#[path = "../../../src/core/work/unit/space/view_binding.rs"]
mod view_binding;
pub(crate) use view_binding::ViewBinding;

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::lock::SpinLock;
use crate::pend::SourceAuthorization;
use env::PieToken;

/// 宿主侧的来源侧登记：只留 `retarget_branch` 真正用到的那两条事实
/// （这条登记属于哪一份来源、它的显式分支索引），操作照真做：同源的才改，只改那一枚。
pub struct Views {
    entries: SpinLock<Vec<(Arc<SourceAuthorization>, Option<PieToken>)>>,
}

impl Views {
    pub fn new() -> Self {
        Self {
            entries: SpinLock::new(Vec::new()),
        }
    }

    /// 记一条登记。
    pub fn note(&self, source: Arc<SourceAuthorization>, branch: Option<PieToken>) {
        self.entries.lock().push((source, branch));
    }

    /// 分支改指：只改这一份来源自己那些仍带着被忘掉那枚 token 的记录。
    pub fn retarget_branch(
        &self,
        source: &Arc<SourceAuthorization>,
        forgotten: PieToken,
        parent: PieToken,
    ) {
        for (held, branch) in self.entries.lock().iter_mut() {
            if Arc::ptr_eq(held, source) && *branch == Some(forgotten) {
                *branch = Some(parent);
            }
        }
    }
}

pub struct Space {
    pub(crate) views: Views,
}

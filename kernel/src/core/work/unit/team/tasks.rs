use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::work::unit::task::Task;
use crate::machine::sync::SpinLock;

use super::Member;

/// 一个团队的成员表：全量成员（发布时登记）与「暂扣」表（已装配但有任务
/// 还没被释放出去）。
pub(crate) struct Tasks {
    pub(crate) members: Arc<SpinLock<Vec<Arc<Member>>>>,
    pub(crate) held: SpinLock<Vec<Arc<Task>>>,
}

impl Tasks {
    /// 暂扣一个任务：同一任务只登记一次（重复停泊是幂等的），容量必须已由发布
    /// 路径留出。
    pub(crate) fn park(&self, task: Arc<Task>) {
        let mut held = self.held.lock();
        if !held.iter().any(|t| Arc::ptr_eq(t, &task)) {
            assert!(
                held.len() < held.capacity(),
                "park capacity reserved before publish"
            );
            held.push(task);
        }
    }

    /// 释放一个任务：从暂扣表里摘掉，回答是否本来就在表上。
    pub(crate) fn release_held(&self, task: &Arc<Task>) -> bool {
        let mut held = self.held.lock();
        if let Some(at) = held.iter().position(|t| Arc::ptr_eq(t, task)) {
            held.swap_remove(at);
            true
        } else {
            false
        }
    }
}

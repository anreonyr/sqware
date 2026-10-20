//! 父来源名下的一条**派生分配槽**：哪一枚分支授权了它、它的几何与子记录弱引用。
//!
//! 槽是父侧记住“这一段已经分出去了”的唯一事实：子记录丢了也不能把它当空闲，
//! 也不能因此封掉整个父范围。它不强持子记录（那会与子强持父形成环），也不持帧；
//! 帧有没有被压着是独立持有自己那条登记的事。

use alloc::sync::{Arc, Weak};

use crate::core::memory::address::{PageCount, PageOffset};
use crate::machine::sync::SpinLock;
use env::PieToken;

use super::geometry::Geometry;
use super::source_authorization::SourceAuthorization;

pub(crate) struct Derivation {
    /// 历史分支索引：授权这一段的父分支 token；Forget 在同一事务里原地改指。
    token: SpinLock<PieToken>,
    geometry: Arc<Geometry>,
    child: Weak<SourceAuthorization>,
}

impl Derivation {
    pub(crate) fn new(
        token: PieToken,
        geometry: Arc<Geometry>,
        child: Weak<SourceAuthorization>,
    ) -> Self {
        Self {
            token: SpinLock::new(token),
            geometry,
            child,
        }
    }

    /// 历史分支索引（自检读口）。
    #[cfg(debug_assertions)]
    pub(crate) fn token(&self) -> PieToken {
        *self.token.lock()
    }

    /// 分支改指：只换历史索引，权限仍是创建时的快照。
    pub(crate) fn retarget(&self, forgotten: PieToken, parent: PieToken) {
        let mut token = self.token.lock();
        if *token == forgotten {
            *token = parent;
        }
    }

    pub(crate) fn geometry(&self) -> &Arc<Geometry> {
        &self.geometry
    }

    /// 仍代表这一段的子记录；升不上就是未知（照旧占住这一段）。
    pub(crate) fn child(&self) -> &Weak<SourceAuthorization> {
        &self.child
    }

    pub(crate) fn overlaps(&self, offset: PageOffset, count: PageCount) -> bool {
        self.geometry.overlaps(offset, count)
    }
}

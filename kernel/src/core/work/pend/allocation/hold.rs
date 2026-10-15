//! 一份**普通引用**：某份来源授权下的一段页窗口与它的访问上限。
//!
//! 引用只登记在**持有它的那个 Task** 的引用表里，所以句柄是表的作用域；
//! 它不搬页、不占 VA、也不持有 `View`（`Release` 因此不隐式 `Detach`）。
//!
//! **它不永久强持来源**：除了带独立保留的那一份，它只记住相对来源授权的页坐标，
//! 真正能用的范围每次向授权记录解析——来源被根 Grant 接管后，撤销即解析不到，
//! 普通引用不会在撤销后继续 pin 住帧。只有独立保留那一份自己强持那段范围，
//! 于是它才能跨 Driver 撤销继续持同一批帧。

use alloc::sync::Arc;

use crate::core::memory::address::{PageCount, PageOffset};
use crate::core::memory::flat::{FlatRange, Residency};
use crate::core::memory::nest::Access;
use crate::core::work::grant::Grant;
use env::PieToken;

use super::source_authorization::SourceAuthorization;

pub(crate) struct Hold {
    id: usize,
    source: Arc<SourceAuthorization>,
    offset: PageOffset,
    count: PageCount,
    ceiling: Access,
    /// 独立保留：自己强持这一段已经解好的范围。
    retained: Option<Residency>,
    /// 这份引用是从**哪条 Grant 分支**取得的（Pole 的哪一枚 Pie）。
    /// 撤销边与上限都按这一条算，不因同 Flat、共享根资格就并到根上去。
    branch: Option<(PieToken, Arc<Grant>)>,
}

impl Hold {
    pub(super) fn new(
        id: usize,
        source: Arc<SourceAuthorization>,
        offset: PageOffset,
        count: PageCount,
        ceiling: Access,
        retained: Option<Residency>,
        branch: Option<(PieToken, Arc<Grant>)>,
    ) -> Self {
        Self {
            id,
            source,
            offset,
            count,
            ceiling,
            retained,
            branch,
        }
    }

    pub(crate) fn id(&self) -> usize {
        self.id
    }

    pub(crate) fn source(&self) -> &Arc<SourceAuthorization> {
        &self.source
    }

    /// 相对来源授权整段的页窗口。
    pub(crate) fn window(&self) -> (PageOffset, PageCount) {
        (self.offset, self.count)
    }

    pub(crate) fn ceiling(&self) -> Access {
        self.ceiling
    }

    pub(crate) fn retains(&self) -> bool {
        self.retained.is_some()
    }

    /// 这条分支（Pole 的 token 与它的 Grant）；私有来源没有。
    pub(crate) fn branch(&self) -> Option<(PieToken, &Arc<Grant>)> {
        self.branch.as_ref().map(|(token, grant)| (*token, grant))
    }

    /// 这条分支的 Grant（没有分支就是 `None`）。
    pub(crate) fn grant(&self) -> Option<&Arc<Grant>> {
        self.branch.as_ref().map(|(_, grant)| grant)
    }

    /// 这一段引用**当前**能拿到的来源范围。
    ///
    /// 独立保留自己强持；其余向来源授权解析：根 Grant 失效就返回 `None`，
    /// 于是未经保留的引用在撤销后既不能挂载也不再 pin 住来源。
    pub(crate) fn resolve(&self) -> Option<FlatRange> {
        if let Some(range) = &self.retained {
            return Some(range.range().clone());
        }
        if let Some((_, grant)) = &self.branch {
            grant.permission()?;
            let (meta, _) = self.source.root()?;
            if !meta.alive() {
                return None;
            }
            return meta.source().range(self.offset, self.count).ok();
        }
        self.source.window(self.offset, self.count)
    }
}

unsafe impl Send for Hold {}
unsafe impl Sync for Hold {}

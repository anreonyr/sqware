use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::nest::NestError;

use super::View;

/// 一段关系上的选择。
///
/// `Whole` 只是一个标记：操作时按“当前活动 ∪ 未结算退休”解析，所以部分撤下之后
/// 剩下的就是还欠着的那几段。`Pages` 是创建时定下的页坐标：它必须整段都还在本
/// 关系的名下（活动或未结算退休），落在已经结算完的洞上就是失败 —— 选择不做
/// 静默过滤，也就不存在“选到已关闭页却当没事”。
#[derive(Clone)]
pub(crate) enum ViewRange {
    Whole(Arc<View>),
    Pages {
        view: Arc<View>,
        offset: PageOffset,
        count: PageCount,
    },
}

impl ViewRange {
    /// 初始范围内的固定页坐标选择；越界或零页即拒。
    pub(super) fn pages(
        view: &Arc<View>,
        offset: PageOffset,
        count: PageCount,
    ) -> Result<Self, NestError> {
        let end = offset
            .get()
            .checked_add(count.get())
            .ok_or(NestError::NotAligned)?;
        if end
            .checked_mul(PAGE_SIZE)
            .is_none_or(|end| end > view.initial_size())
        {
            return Err(NestError::NoRegion);
        }
        Ok(Self::Pages {
            view: view.clone(),
            offset,
            count,
        })
    }

    pub(crate) fn view(&self) -> &Arc<View> {
        match self {
            Self::Whole(view) => view,
            Self::Pages { view, .. } => view,
        }
    }

    /// 选择的字节窗口（含尾）。`Whole` 取整个初始范围；含尾表示因此顶页也不溢出。
    pub(crate) fn window(&self) -> (usize, usize) {
        match self {
            Self::Whole(view) => {
                let start = view.initial_start().as_usize();
                (start, start + (view.initial_size() - 1))
            }
            Self::Pages {
                view,
                offset,
                count,
            } => {
                let start = view.initial_start().as_usize() + offset.get() * PAGE_SIZE;
                (start, start + (count.get() * PAGE_SIZE - 1))
            }
        }
    }

    /// 本选择窗口内的活动片段（含端点、地址升序）。
    ///
    /// 派生编排显式地要“源活动 ∩ 窗口”：这里把交集当结果返回，不是核心操作
    /// 对已关闭页的静默过滤。
    pub(crate) fn active_fragments(&self) -> Result<Vec<(VirtAddr, usize)>, NestError> {
        let (lo, hi) = self.window();
        let active = self.view().active()?;
        let mut fragments = Vec::new();
        fragments
            .try_reserve(active.len())
            .map_err(|_| NestError::OutOfMemory)?;
        for range in active {
            if let Some((lo, hi)) = range.intersection(lo, hi) {
                fragments.push((VirtAddr::from_raw(lo), hi - lo + 1));
            }
        }
        Ok(fragments)
    }

    /// 源在本选择内的子范围：强持同一来源、不复制数据。
    pub(crate) fn source(&self) -> Result<FlatRange, NestError> {
        let view = self.view();
        let (offset, count) = match self {
            Self::Whole(_) => (
                PageOffset::ZERO,
                PageCount::new(view.initial_size() / PAGE_SIZE).ok_or(NestError::NotAligned)?,
            ),
            Self::Pages { offset, count, .. } => (*offset, *count),
        };
        Ok(view.source().range(offset, count)?)
    }
}

// 区间分配器 — 段表 + 访问器（accessor）：一个 IntervalInner 自持多段，访问器
// 内涵段信息（Arc 共享段表 + 段索引）。段内统一 lowest first-fit——无方向、
// 无偏好。
//
// 中心意象：**返回地址区间**（管理未映射 VA 窗口，不实现 `core::alloc::Allocator`）；
// 记账按**存活块数**（每块一条 BTreeMap 条目）——窗口再大也零 up-front 成本。
// 段内互不重叠（首次适配保证），段间互不重叠由各段 `[base, edge)` 几何隔离
// （free 段与 frame 段分处用户/内核半区，见 layout.rs `validate`）。
//
// 用法：`register` 注册一段即得绑定访问器（同一个 Arc 共享段表）——窗口/借用方
// 持访问器取段/还段，不再路由段；段表随 Space 同生共死，访问器不晚于段表 drop。

use alloc::alloc::AllocError;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::lock::SpinLock;

/// 段访问器 — 内涵段信息（Arc 共享段表 + 段索引）。[`seg`] 由 [`register`]
/// 产出（段表 push 序），恒合法；可复制（同段多持有者共享同一段表）。
#[derive(Clone)]
pub(crate) struct IntervalAllocator {
    /// 共享段表（一个 Space 一张）。
    inner: Arc<SpinLock<IntervalInner>>,
    /// 本访问器绑定段的索引（段表 push 序）。
    seg: usize,
}

impl core::fmt::Debug for IntervalAllocator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IntervalAllocator")
            .field("seg", &self.seg)
            .finish()
    }
}

/// 段表（一个 Space 一张）— 段区间表 + 每段已分配区间表（平行索引）。
#[derive(Debug)]
pub(crate) struct IntervalInner {
    /// 段表：`segments[i] = (base, edge)`（[base, edge)）。
    segments: Vec<(usize, usize)>,
    /// 每段已分配区间表：`allocated[i]` = 起始地址 → 长度（段内键序互斥）。
    allocated: Vec<BTreeMap<usize, usize>>,
}

impl IntervalInner {
    /// 构造（空段表；段经 [`register`] 注册）。
    pub(crate) fn new() -> Self {
        Self {
            segments: Vec::new(),
            allocated: Vec::new(),
        }
    }
}

/// 注册一段 `[base, edge)`，返回绑定该段的访问器（Arc 共享同表）。
///
/// 每次调用新增**一段**（push 新段表条目）——同一段的多持有者应共享
/// 同一个访问器（Clone），不要重复注册。
pub(crate) fn register(
    core: &Arc<SpinLock<IntervalInner>>,
    base: usize,
    edge: usize,
) -> IntervalAllocator {
    let mut g = core.lock();
    let seg = g.segments.len();
    g.segments.push((base, edge));
    g.allocated.push(BTreeMap::new());
    IntervalAllocator {
        inner: core.clone(),
        seg,
    }
}

impl IntervalAllocator {
    /// 本段 lowest first-fit：沿本段分配表扫空隙，取**最低**够大者落块。
    ///
    /// 返回块基址；`size == 0` 按 1 处理。
    ///
    /// # Errors
    ///
    /// 段内无足够连续空隙 → [`AllocError`]。
    pub(crate) fn allocate(&self, size: usize) -> Result<usize, AllocError> {
        let size = size.max(1);
        let mut g = self.inner.lock();
        let (base, edge) = g.segments[self.seg];
        let tab = &mut g.allocated[self.seg];
        let mut cursor = base;
        for (&start, &len) in tab.iter() {
            // 空隙 [cursor, start)：候选
            if start.saturating_sub(cursor) >= size {
                tab.insert(cursor, size);
                return Ok(cursor);
            }
            cursor = cursor.max(start.saturating_add(len));
        }
        // 段尾空隙 [cursor, edge)
        if edge.saturating_sub(cursor) >= size {
            tab.insert(cursor, size);
            return Ok(cursor);
        }
        Err(AllocError)
    }

    /// 精确匹配释放 `(addr, size)`：条目存在且长度相等 → 删除 → `true`。
    ///
    /// 未分配 / 长度不匹配 / 越界 → `false`。
    pub(crate) fn deallocate(&self, addr: usize, size: usize) -> bool {
        let mut g = self.inner.lock();
        let tab = &mut g.allocated[self.seg];
        match tab.get(&addr) {
            Some(&len) if len == size => {
                tab.remove(&addr);
                true
            }
            _ => false,
        }
    }
}

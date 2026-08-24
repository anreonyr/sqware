// 区间分配器 — 以起始地址为键的有序区间树（BTreeMap）上的连续区间分配。
//
// 语义对齐分配器家族（block/frame/bitmap）：`allocate` / `deallocate` + `AllocError`，
// 但返回的是**地址区间**（管理未映射 VA 窗口，不实现 `core::alloc::Allocator`）。
//
// 与位图（bitmap）不同之处只在记账结构：位图付费给窗口容量（整窗物化），
// 本分配器付费给**存活块数**（每块一条区间树条目）——窗口大到用户半区
// （Sv48/Sv57 的 TiB 级堆窗口）也零 up-front 成本。allocated 区间互不重叠
// （首次适配保证），故区间树即「已分配区间集合」。
//
// 找洞 = 有序树序遍历间隙（first-fit）；释放 = 精确键查 `(addr, size)` 匹配删除。

use alloc::alloc::AllocError;
use alloc::collections::BTreeMap;

/// 区间分配器 — 在编号空间 `[base, edge)` 上按任意字节区间分配连续块。
///
/// # 不变量
///
/// - allocated 各条目互不重叠、升序有序（BTreeMap 键序保证）。
/// - 分配区间恒落在 `[base, edge)` 内。
#[derive(Debug)]
pub(crate) struct IntervalAllocator {
    /// 空间基址（含）。
    base: usize,
    /// 空间上界（不含）。
    edge: usize,
    /// 已分配区间表：起始地址 → 长度。
    allocated: BTreeMap<usize, usize>,
}

/// 取段方向 — 空隙选择的倾向（隐喻作值：Rise=低端优先，Fall=高端优先）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// 底向上：取**最低**够大空隙（堆/帧等低端生长）。
    Rise,
    /// 顶向下：取**最高**够大空隙，块落空隙高端（栈槽自顶向下排、mmap 高位取段）。
    Fall,
}

impl IntervalAllocator {
    /// 构造（空区间树；`BTreeMap::new` 为 const，静态实例可行）。
    pub(crate) const fn new(base: usize, edge: usize) -> Self {
        Self {
            base,
            edge,
            allocated: BTreeMap::new(),
        }
    }

    /// 按方向取段：沿有序树序遍历空隙，按 [`Direction`] 选最低/最高够大空隙落块。
    ///
    /// 返回 `(区间基址, size)`；`size == 0` 按 1 处理。
    ///
    /// # Errors
    ///
    /// 空间耗尽或窗口内无足够连续空隙 → [`AllocError`]。
    pub(crate) fn allocate(
        &mut self,
        size: usize,
        dir: Direction,
    ) -> Result<(usize, usize), AllocError> {
        let size = size.max(1);
        match dir {
            Direction::Rise => {
                let mut cursor = self.base;
                for (&start, &len) in self.allocated.iter() {
                    // 空隙 [cursor, start)：候选
                    if start.saturating_sub(cursor) >= size {
                        self.allocated.insert(cursor, size);
                        return Ok((cursor, size));
                    }
                    cursor = cursor.max(start.saturating_add(len));
                }
                // 窗口尾空隙 [cursor, edge)
                if self.edge.saturating_sub(cursor) >= size {
                    self.allocated.insert(cursor, size);
                    return Ok((cursor, size));
                }
                Err(AllocError)
            }
            Direction::Fall => {
                let mut cursor = self.edge;
                for (&start, &len) in self.allocated.iter().rev() {
                    // 空隙 [start+len, cursor)：候选，块落空隙高端
                    let gap_lo = start.saturating_add(len);
                    if cursor.saturating_sub(gap_lo) >= size {
                        let base = cursor - size;
                        self.allocated.insert(base, size);
                        return Ok((base, size));
                    }
                    cursor = start;
                }
                // 窗口底空隙 [base, cursor)
                if cursor.saturating_sub(self.base) >= size {
                    let base = cursor - size;
                    self.allocated.insert(base, size);
                    return Ok((base, size));
                }
                Err(AllocError)
            }
        }
    }

    /// 精确匹配释放 `(addr, size)`：条目存在且长度相等 → 删除 → `Ok(())`。
    ///
    /// # Errors
    ///
    /// 未分配 / 长度不匹配 / 越界 → [`AllocError`]。
    pub(crate) fn deallocate(&mut self, addr: usize, size: usize) -> Result<(), AllocError> {
        match self.allocated.get(&addr) {
            Some(&len) if len == size => {
                self.allocated.remove(&addr);
                Ok(())
            }
            _ => Err(AllocError),
        }
    }
}

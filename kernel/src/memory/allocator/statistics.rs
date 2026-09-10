//! 分配子系统统一统计出口。
//!
//! 三个分配器（frame / block / spare）各自**不再保留** occupied / available /
//! pages / used 等 read-only 统计字段（无算法依赖）；record 钩子在持分配器
//! 锁期间同步增减本模块的 atomic，作为唯一权威。
//!
//! 视图层 [`FrameView`] / [`BlockView`] / [`SpareView`] / [`Delta`] / [`Baseline`]
//! 由本模块 init 时分配驻留内存，pub fn 返回 `&'static` 引用，调用方零拷贝。
//!
//! 命名贯穿：统计内部字段路径与视图字段名同根。
//!
//! 可见性说明：读侧（view_* / Delta / Baseline）的消费者是 audit 与 debug 自检，
//! 故默认 release 构建下未被调用属**预期**——需要时由消费方接入即活跃，
//! 不使用模块级 blanket allow 压制。

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;

use crate::lock::OnceLock;
#[cfg(feature = "audit")]
use crate::lock::RwLock;

use super::fence::{KIND_COUNT, Kind};
use super::spare;

/// 预留池数上限（risc-v 主流硬件 ≤ 8 核；超出则统计只取前 8 个池）。
pub(super) const MAX_POOLS: usize = 8;
// 种类计数按 fence::KIND_COUNT 定长（对象种类是记账的唯一维度，见 fence::kind）。

// ── 视图类型 ──

#[derive(Clone, Copy)]
pub struct FrameView {
    pub total: usize,
    pub available: usize,
    pub occupied: usize,
    pub kinds: [usize; KIND_COUNT],
}

#[derive(Clone, Copy)]
pub struct PoolStat {
    pub id: usize,
    pub pages: usize,
    pub freepool_total: usize,
}

#[derive(Clone, Copy)]
pub struct BlockView {
    pub pools: [PoolStat; MAX_POOLS],
    pub occupied: usize,
    pub kinds: [usize; KIND_COUNT],
}

#[derive(Clone, Copy)]
pub struct SpareView {
    pub total: usize,
    pub occupied: usize,
    pub available: usize,
    pub dump_budget: usize,
}

#[cfg(feature = "audit")]
#[derive(Clone, Copy)]
pub struct BaselineFrame {
    pub total: usize,
    pub occupied: usize,
}

#[cfg(feature = "audit")]
#[derive(Clone, Copy)]
pub struct BaselineBlock {
    pub occupied: usize,
}

#[cfg(feature = "audit")]
#[derive(Clone, Copy)]
pub struct BaselineSpare {
    pub total: usize,
    pub occupied: usize,
}

#[cfg(feature = "audit")]
#[derive(Clone, Copy)]
pub struct Baseline {
    pub frame: BaselineFrame,
    pub block: BaselineBlock,
    pub spare: BaselineSpare,
}

#[derive(Clone, Copy)]
pub struct FrameDiff {
    pub total: isize,
    pub available: isize,
    pub occupied: isize,
}

#[derive(Clone, Copy)]
pub struct BlockDiff {
    pub occupied: isize,
}

#[derive(Clone, Copy)]
pub struct SpareDiff {
    pub total: isize,
    pub occupied: isize,
    pub available: isize,
}

#[derive(Clone, Copy)]
pub struct Delta {
    pub frame: FrameDiff,
    pub block: BlockDiff,
    pub spare: SpareDiff,
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    pub frame: FrameView,
    pub block: BlockView,
    pub spare: SpareView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotInitialized,
    AlreadyInitialized,
}

// ── 内部驻留 ──

struct FrameStats {
    total: AtomicUsize,
    available: AtomicUsize,
    occupied: AtomicUsize,
    kinds: [AtomicUsize; KIND_COUNT],
}

struct BlockStats {
    occupied: AtomicUsize,
    pools: [AtomicUsize; MAX_POOLS],
    kinds: [AtomicUsize; KIND_COUNT],
}

struct SpareStats {
    total: AtomicUsize,
    occupied: AtomicUsize,
    available: AtomicUsize,
}

struct Stats {
    frame: FrameStats,
    block: BlockStats,
    spare: SpareStats,
    #[cfg(feature = "audit")]
    baseline_lock: RwLock<Baseline>,
    frame_view_cell: UnsafeCell<FrameView>,
    block_view_cell: UnsafeCell<BlockView>,
    spare_view_cell: UnsafeCell<SpareView>,
}

// SAFETY: Stats 内的 UnsafeCell 访问均通过 `unsafe { &mut *ptr.get() }` 单线程
// 调用方独占（pub fn 在 &Stats 上调用,Stats 本身不可变借用），并发安全由 RwLock
// 与 AtomicUsize 的内部同步保证；UnsafeCell 仅作为「独占可写容器」，不提供跨
// 线程的内部可变性（视图构建串行发生在 pub fn 内，不并发）。
unsafe impl Sync for Stats {}

static STATS: OnceLock<&'static Stats> = OnceLock::new();

fn stats() -> &'static Stats {
    STATS.get().expect("statistics not initialized")
}

const ZERO_FRAME_VIEW: FrameView = FrameView {
    total: 0,
    available: 0,
    occupied: 0,
    kinds: [0; KIND_COUNT],
};

const ZERO_POOL_STAT: PoolStat = PoolStat {
    id: 0,
    pages: 0,
    freepool_total: 0,
};

const ZERO_BLOCK_VIEW: BlockView = BlockView {
    pools: [ZERO_POOL_STAT; MAX_POOLS],
    occupied: 0,
    kinds: [0; KIND_COUNT],
};

const ZERO_SPARE_VIEW: SpareView = SpareView {
    total: 0,
    occupied: 0,
    available: 0,
    dump_budget: 0,
};

#[cfg(feature = "audit")]
const ZERO_BASELINE: Baseline = Baseline {
    frame: BaselineFrame {
        total: 0,
        occupied: 0,
    },
    block: BaselineBlock { occupied: 0 },
    spare: BaselineSpare {
        total: 0,
        occupied: 0,
    },
};

// ── 装配 ──

pub fn init() -> Result<(), Error> {
    if STATS.get().is_some() {
        return Err(Error::AlreadyInitialized);
    }
    let s: &'static Stats = Box::leak(Box::new(Stats {
        frame: FrameStats {
            total: AtomicUsize::new(0),
            available: AtomicUsize::new(0),
            occupied: AtomicUsize::new(0),
            kinds: [const { AtomicUsize::new(0) }; KIND_COUNT],
        },
        block: BlockStats {
            occupied: AtomicUsize::new(0),
            pools: [const { AtomicUsize::new(0) }; MAX_POOLS],
            kinds: [const { AtomicUsize::new(0) }; KIND_COUNT],
        },
        spare: SpareStats {
            total: AtomicUsize::new(0),
            occupied: AtomicUsize::new(0),
            available: AtomicUsize::new(0),
        },
        #[cfg(feature = "audit")]
        baseline_lock: RwLock::new(ZERO_BASELINE),
        frame_view_cell: UnsafeCell::new(ZERO_FRAME_VIEW),
        block_view_cell: UnsafeCell::new(ZERO_BLOCK_VIEW),
        spare_view_cell: UnsafeCell::new(ZERO_SPARE_VIEW),
    }));
    STATS.set(s).map_err(|_| Error::AlreadyInitialized)?;
    #[cfg(feature = "audit")]
    rebaseline()?;
    Ok(())
}

#[cfg(feature = "audit")]
pub fn rebaseline() -> Result<(), Error> {
    let s = stats();
    let cur = Baseline {
        frame: BaselineFrame {
            total: s.frame.total.load(Ordering::Relaxed),
            occupied: s.frame.occupied.load(Ordering::Relaxed),
        },
        block: BaselineBlock {
            occupied: s.block.occupied.load(Ordering::Relaxed),
        },
        spare: BaselineSpare {
            total: s.spare.total.load(Ordering::Relaxed),
            occupied: s.spare.occupied.load(Ordering::Relaxed),
        },
    };
    *s.baseline_lock.write() = cur;
    Ok(())
}

// ── record 钩子（pub(super) 由 frame/block/spare 内部 alloc/free 调）──

pub(crate) fn record_frame_take(kind: Kind) {
    let s = stats();
    s.frame.occupied.fetch_add(1, Ordering::Relaxed);
    s.frame.kinds[kind as usize].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_frame_give(kind: Kind) {
    let s = stats();
    s.frame.occupied.fetch_sub(1, Ordering::Relaxed);
    s.frame.kinds[kind as usize].fetch_sub(1, Ordering::Relaxed);
}

#[cfg(feature = "audit")]
pub(crate) fn record_frame_relabel(from: Kind, to: Kind) {
    let s = stats();
    s.frame.kinds[from as usize].fetch_sub(1, Ordering::Relaxed);
    s.frame.kinds[to as usize].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_frame_total(total: usize) {
    if let Some(s) = STATS.get() {
        s.frame.total.store(total, Ordering::Relaxed);
    }
}

pub(crate) fn record_frame_available(available: usize) {
    if let Some(s) = STATS.get() {
        s.frame.available.store(available, Ordering::Relaxed);
    }
}

pub(crate) fn record_pool_take(pool_id: usize) {
    let s = stats();
    s.block.occupied.fetch_add(1, Ordering::Relaxed);
    if let Some(p) = s.block.pools.get(pool_id) {
        p.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn record_pool_give(pool_id: usize) {
    let s = stats();
    s.block.occupied.fetch_sub(1, Ordering::Relaxed);
    if let Some(p) = s.block.pools.get(pool_id) {
        p.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(feature = "audit")]
pub(crate) fn record_block_relabel(from: Kind, to: Kind) {
    let s = stats();
    s.block.kinds[from as usize].fetch_sub(1, Ordering::Relaxed);
    s.block.kinds[to as usize].fetch_add(1, Ordering::Relaxed);
}

/// fence::on_alloc / retire 用：块按种类 +1（仅维护 kinds 数组，不动 block.occupied——
/// block.occupied 由 prime/drain 路径维护,反映块池借出页数,与种类计数维度不同）。
#[cfg(feature = "audit")]
pub(crate) fn record_block_take(kind: Kind) {
    if let Some(s) = STATS.get() {
        s.block.kinds[kind as usize].fetch_add(1, Ordering::Relaxed);
    }
}

/// fence::on_free / retire 用：块按种类 -1。
#[cfg(feature = "audit")]
pub(crate) fn record_block_give(kind: Kind) {
    if let Some(s) = STATS.get() {
        s.block.kinds[kind as usize].fetch_sub(1, Ordering::Relaxed);
    }
}

// record_block_pool_freepool 当前未启用——freepool 块数通过 view_block() 时
// 现场遍历 freepool[power] 累加（per-power 块数不频繁统计,不维护 atomic 副本）。
// 留空位以便未来需要时启用。

pub(crate) fn record_spare_take(bytes: usize) {
    stats().spare.occupied.fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_give(bytes: usize) {
    stats().spare.occupied.fetch_sub(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_total(total: usize) {
    if let Some(s) = STATS.get() {
        s.spare.total.store(total, Ordering::Relaxed);
    }
}

pub(crate) fn record_spare_available(available: usize) {
    if let Some(s) = STATS.get() {
        s.spare.available.store(available, Ordering::Relaxed);
    }
}

// ── 视图（&'static 引用返回；内部 UnsafeCell 装最新视图，调用方零拷贝）──

pub fn view_frame() -> &'static FrameView {
    let s = stats();
    let v = unsafe { &mut *s.frame_view_cell.get() };
    v.total = s.frame.total.load(Ordering::Relaxed);
    v.available = s.frame.available.load(Ordering::Relaxed);
    v.occupied = s.frame.occupied.load(Ordering::Relaxed);
    for i in 0..KIND_COUNT {
        v.kinds[i] = s.frame.kinds[i].load(Ordering::Relaxed);
    }
    v
}

pub fn view_block() -> &'static BlockView {
    let s = stats();
    let v = unsafe { &mut *s.block_view_cell.get() };
    v.occupied = s.block.occupied.load(Ordering::Relaxed);
    for i in 0..KIND_COUNT {
        v.kinds[i] = s.block.kinds[i].load(Ordering::Relaxed);
    }
    for (i, pool) in v.pools.iter_mut().enumerate() {
        pool.id = i;
        pool.pages = s.block.pools[i].load(Ordering::Relaxed);
        pool.freepool_total = 0; // TODO: 现场聚合 freepool[power] 块数
    }
    v
}

pub fn view_spare() -> &'static SpareView {
    let s = stats();
    let v = unsafe { &mut *s.spare_view_cell.get() };
    v.total = s.spare.total.load(Ordering::Relaxed);
    v.occupied = s.spare.occupied.load(Ordering::Relaxed);
    v.available = s.spare.available.load(Ordering::Relaxed);
    v.dump_budget = spare::DUMP_BUDGET;
    v
}

#[cfg(feature = "audit")]
pub fn delta() -> Result<Delta, Error> {
    let s = stats();
    let bl = *s.baseline_lock.read();
    let cur = Snapshot {
        frame: *view_frame(),
        block: *view_block(),
        spare: *view_spare(),
    };
    Ok(Delta {
        frame: FrameDiff {
            total: cur.frame.total as isize - bl.frame.total as isize,
            available: cur.frame.available as isize
                - (bl.frame.total as isize - bl.frame.occupied as isize),
            occupied: cur.frame.occupied as isize - bl.frame.occupied as isize,
        },
        block: BlockDiff {
            occupied: cur.block.occupied as isize - bl.block.occupied as isize,
        },
        spare: SpareDiff {
            total: cur.spare.total as isize - bl.spare.total as isize,
            occupied: cur.spare.occupied as isize - bl.spare.occupied as isize,
            available: cur.spare.available as isize
                - (bl.spare.total as isize - bl.spare.occupied as isize),
        },
    })
}

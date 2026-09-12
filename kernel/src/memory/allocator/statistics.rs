//! 分配子系统统一统计出口。
//!
//! 三个分配器（frame / block / spare）各自**不再保留** occupied / available 等
//! read-only 统计字段（无算法依赖）；record 钩子在持分配器锁期间同步增减本模块的
//! atomic，作为唯一权威。
//!
//! # 只留**判据输入**
//!
//! 这里只回答四笔总数：帧池在手的帧数、块分配器从帧池借走的页数、后备仓在手的字节数
//! 与余量。它们是 `health/` 三条用例的判据输入（"净漏帧" / "仓预算演练闭环"），
//! **没有别的东西读**——曾经有过的 `total` / `available` 水位、逐池页数、关机收尾
//! 用的 `Baseline` / `Delta` 三件套都属于审计层，随那一层一起删了：留下没人读的
//! 读数只会让"这里还有个判据"变成错觉。
//!
//! 消费者只在 debug / framework 档存在（`health/pagetable.rs` 的用例有同一个
//! gate），故本模块的读侧在 release 档下**全部**是死的——这是构造上的必然，不是
//! 遗漏：想让某个数在别的档也被读，就在那一档加一条真读它的判据。
//!
//! # 类目（`Kind` / `tag!` / 逐类在册数）
//!
//! 除了那四笔总数，本模块还收着"**这一笔分配是干什么用的**"这一整套：词表
//! [`Kind`]、标注 [`mark`]、两张类目表（帧侧一帧一格、块侧一格 256 B）、逐类在册数
//! [`kinds`]，以及那枚把它们串起来的宏 [`tag!`]。**判据的把手**是最后一项：净额非 0
//! 时，逐类读数直接说出漏的是哪一类，不必从总量反推机制。
//!
//! 两张表**不合并**：粒度不同，且会互相覆盖（一个 `Prime` 池页里再有一个 `Task` 块，
//! 池页归还时按首格读会读到 `Task` ⇒ 计数漂移）。计数**合并成一份**：判据是逐类净额，
//! 与容器无关，合成一份就不必为"哪一侧"再造一条读数。
//!
//! 这一整套只在 debug / framework 档存在（与读侧同一个 gate）：
//! · release 档 `Kind` 连类型都不存在——宏恒等展开，分配器调用点跨档同形；
//! · 两张类目表与标注格都不进产物（release 下不装配、整块 cfg out），热路径在
//!   release 档只剩原来那两笔 RMW。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;
#[cfg(any(debug_assertions, feature = "framework"))]
use alloc::vec::Vec;

use crate::lock::OnceLock;

// ── 类目：词表 ──────────────────────────────────────────

/// 分配对象的**类**：这一笔分配是干什么用的。
///
/// `Plain` = 未标注（容器增长、大块直取、任何没说清自己的分配）——0 = 表全零，
/// 故"未标注"就是 0，不占额外状态。词表次序 = 判别式次序：`ALL`、计数数组、
/// 打印次序都从它一处出。
#[cfg(any(debug_assertions, feature = "framework"))]
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Plain = 0,
    Trap,
    Lazy,
    Heap,
    Stack,
    Image,
    Ring,
    Table,
    TrapStack,
    HartFrame,
    Spare,
    Prime,
    Probe,
    // ── 块侧：内核原语的外壳（`Arc<Task>` / `Arc<Team>` / `Arc<Space>`）──
    Task,
    Team,
    Space,
}

#[cfg(any(debug_assertions, feature = "framework"))]
impl Kind {
    /// 类的个数（计数数组与 `ALL` 的长度）。
    pub(crate) const COUNT: usize = 16;

    /// 全部类（次序 = 判别式 = 打印次序）。
    pub(crate) const ALL: [Kind; Kind::COUNT] = [
        Kind::Plain,
        Kind::Trap,
        Kind::Lazy,
        Kind::Heap,
        Kind::Stack,
        Kind::Image,
        Kind::Ring,
        Kind::Table,
        Kind::TrapStack,
        Kind::HartFrame,
        Kind::Spare,
        Kind::Prime,
        Kind::Probe,
        Kind::Task,
        Kind::Team,
        Kind::Space,
    ];

    /// 下标 = 判别式（计数数组按它索引）。
    pub(crate) const fn ix(self) -> usize {
        self as usize
    }

    /// 读数里用的名字（两处读数共用这一份，不另维字符串表）。
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Trap => "trap",
            Kind::Lazy => "lazy",
            Kind::Heap => "heap",
            Kind::Stack => "stack",
            Kind::Image => "image",
            Kind::Ring => "ring",
            Kind::Table => "table",
            Kind::TrapStack => "trap-stack",
            Kind::HartFrame => "hart-frame",
            Kind::Spare => "spare",
            Kind::Prime => "prime",
            Kind::Probe => "probe",
            Kind::Task => "task",
            Kind::Team => "team",
            Kind::Space => "space",
        }
    }
}

// ── 类目：标注（本核当前标注 + 作用域守卫）────────────────

/// 每核一格：本核当前标注。
#[cfg(any(debug_assertions, feature = "framework"))]
struct MarkCell(core::cell::Cell<Kind>);

// SAFETY: 每核只碰自己那一格（`mark_slot` 按 `hart_id` 取），且标注的"设—读—恢复"
// 在同一 hart 上完成、不跨挂起（分配路径不自旋外等待）⇒ 没有并发访问。
#[cfg(any(debug_assertions, feature = "framework"))]
unsafe impl Sync for MarkCell {}

#[cfg(any(debug_assertions, feature = "framework"))]
static MARK: [MarkCell; crate::machine::MAX_HART_SLOTS] =
    [const { MarkCell(core::cell::Cell::new(Kind::Plain)) }; crate::machine::MAX_HART_SLOTS];

#[cfg(any(debug_assertions, feature = "framework"))]
fn mark_slot() -> &'static MarkCell {
    // 槽位随 `MAX_HART_SLOTS`（VA 布局表达上限）；实际核数由 DTB 决定，二者同界。
    &MARK[crate::machine::hart_id().min(crate::machine::MAX_HART_SLOTS - 1)]
}

/// 进入一段**标注作用域**：把本核当前标注置为 `kind`，守卫析构时恢复前一值。
///
/// 前置（契约，不是建议）：置与读必须**同核**、且**不跨挂起**——分配路径不自旋外
/// 等待。这两条决定标记格不必关中断、也不必用原子。
#[cfg(any(debug_assertions, feature = "framework"))]
#[must_use = "标注靠守卫的生存期起作用：立刻丢掉等于没标"]
pub(crate) fn mark(kind: Kind) -> Mark {
    Mark {
        prev: mark_slot().0.replace(kind),
    }
}

/// 本核当前标注（没标过 = [`Kind::Plain`]）。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) fn current() -> Kind {
    mark_slot().0.get()
}

/// 标注作用域的守卫（析构即恢复）。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) struct Mark {
    prev: Kind,
}

#[cfg(any(debug_assertions, feature = "framework"))]
impl Drop for Mark {
    fn drop(&mut self) {
        mark_slot().0.set(self.prev);
    }
}

/// 把一笔分配标成某一类：`tag!(Stack, SpaceInner::frame()?)`。
///
/// 它是一条**作用域**：表达式求值期间本核当前标注是 `Kind::$kind`，求值完（含 `?`
/// 早退、panic 中止）恢复前一值；内层没再标就继承外层。
///
/// 只在 debug / framework 档存在；其余档恒等展开（零开销，也不留死词表）。
#[cfg(any(debug_assertions, feature = "framework"))]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {{
        let _m = $crate::memory::allocator::statistics::mark(
            $crate::memory::allocator::statistics::Kind::$kind,
        );
        $v
    }};
}

#[cfg(not(any(debug_assertions, feature = "framework")))]
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {
        $v
    };
}

// ── 类目：每帧类目表 ────────────────────────────────────

/// 每帧类目表句柄：一帧一格，`install_frame_kinds` 装配一次（尺寸 = 帧数）。
///
/// 它回答的是"这帧**当初按哪一类**取走的"——**不是**第二份"在不在手"的账：后者仍
/// 只有 `frame::pagemeta` 一份。读写都发生在持 `FrameAllocator::inner` 锁期间
/// （[`record_frame_take`] / [`record_frame_give`] 的契约），故不必另加同步。
#[cfg(any(debug_assertions, feature = "framework"))]
struct FrameKinds {
    ptr: *mut Kind,
    len: usize,
}

// SAFETY: 句柄只在装配时写入一次，此后 `ptr`/`len` 不变；所指内存只经
// `record_frame_take`/`record_frame_give` 在帧分配器锁内读写。
#[cfg(any(debug_assertions, feature = "framework"))]
unsafe impl Send for FrameKinds {}
#[cfg(any(debug_assertions, feature = "framework"))]
unsafe impl Sync for FrameKinds {}

#[cfg(any(debug_assertions, feature = "framework"))]
static FRAME_KINDS: OnceLock<FrameKinds> = OnceLock::new();

/// 装配每帧类目表（帧数要到 `frame::init` 才知道）。非 debug 档 no-op。
///
/// # Errors
///
/// - 类目表分配失败 → [`InitError::OutOfMemory`]。
/// - 重复装配 → [`InitError::AlreadyInitialized`]。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) fn install_frame_kinds(frames: usize) -> Result<(), super::InitError> {
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(frames)
        .map_err(|_| super::InitError::OutOfMemory)?;
    v.resize(frames, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = FrameKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
    };
    FRAME_KINDS
        .set(h)
        .map_err(|_| super::InitError::AlreadyInitialized)
}

#[cfg(not(any(debug_assertions, feature = "framework")))]
pub(crate) fn install_frame_kinds(_frames: usize) -> Result<(), super::InitError> {
    Ok(())
}

/// 类目表（读写都经下面两个助手；调用方持帧分配器锁 ⇒ 独占）。
#[cfg(any(debug_assertions, feature = "framework"))]
fn frame_kind_table() -> &'static mut [Kind] {
    let h = FRAME_KINDS
        .get()
        .expect("frame kinds not installed (frame::init 应先调 install_frame_kinds)");
    // SAFETY: 指针来自 `Box::leak`（`len` 格，终身有效）；独占性由调用方持帧锁保证
    // （见 `FrameKinds` 的 SAFETY）。
    unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) }
}

/// 写下从 `index` 起 `frames` 帧的类目（一笔分配一写）。
#[cfg(any(debug_assertions, feature = "framework"))]
fn note_frame_kind(index: usize, frames: usize, k: Kind) {
    frame_kind_table()[index..index + frames].fill(k);
}

/// 读 `index` 这一帧的类目（归还时按块首帧问"当初是哪一类"）。
#[cfg(any(debug_assertions, feature = "framework"))]
fn frame_kind_at(index: usize) -> Kind {
    frame_kind_table()[index]
}

// ── 类目：每块类目表（块侧；一格 = `BLOCK_KIND_GRAIN` 字节）────────

/// 块类目表的粒度位移：一格 = `1 << 7` = 128 B。
///
/// **为什么是 7 而不是 8**（实测定的）：内核原语的三类外壳里最小的那个是
/// `Arc<Task>`（`ArcInner<Task>` 128 B = `power 7`）；阈值取 8 会把"任务外壳"这一类
/// 正好漏在表外——而它就是 `leak: task 1` 那一族要点的名。取 7 的代价是表从
/// `区/256` 涨到 `区/128`（16 MiB 区 ⇒ 128 KiB，0.78%）。
///
/// `power < BLOCK_KIND_SHIFT` 的块（≤ 64 B）比一格还小，落不进去：那些块**两侧都按
/// `Plain` 记**（take 不记它的标注、give 也记 `Plain`）——两侧必须对称，否则 take 记了
/// 标注、give 读回 `plain`，逐类账会一笔一笔地漂（这正是本条从 8 改成 7 时当场量的
/// 现象：`task` 每轮 +1、`plain` 每轮 −1）。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) const BLOCK_KIND_SHIFT: usize = 7;

/// 每块类目表句柄：覆盖块分配器的 free 区（与 `Tally` 同一 `base`），
/// `slot = (addr - base) >> BLOCK_KIND_SHIFT`。一页可容多格 ⇒ 一格里可容多个小块，
/// 故与帧侧同形：**取时写满该块覆盖的格，还时读首格**。
#[cfg(any(debug_assertions, feature = "framework"))]
struct BlockKinds {
    ptr: *mut Kind,
    len: usize,
    base: usize,
}

// SAFETY: 同 `FrameKinds` —— 句柄装配一次后不变；所指内存只经
// `record_block_take`/`record_block_give` 在池锁内（`pull`/`push` 路径）读写。
#[cfg(any(debug_assertions, feature = "framework"))]
unsafe impl Send for BlockKinds {}
#[cfg(any(debug_assertions, feature = "framework"))]
unsafe impl Sync for BlockKinds {}

#[cfg(any(debug_assertions, feature = "framework"))]
static BLOCK_KINDS: OnceLock<BlockKinds> = OnceLock::new();

/// 装配块类目表（覆盖 `[base, base + len)` 的字节区间）。非 debug 档 no-op。
///
/// # Errors
///
/// - 类目表分配失败 → [`InitError::OutOfMemory`]。
/// - 重复装配 → [`InitError::AlreadyInitialized`]。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) fn install_block_kinds(base: usize, len: usize) -> Result<(), super::InitError> {
    let slots = len >> BLOCK_KIND_SHIFT;
    let mut v: Vec<Kind> = Vec::new();
    v.try_reserve(slots)
        .map_err(|_| super::InitError::OutOfMemory)?;
    v.resize(slots, Kind::Plain);
    let slice: &'static mut [Kind] = Box::leak(v.into_boxed_slice());
    let h = BlockKinds {
        ptr: slice.as_mut_ptr(),
        len: slice.len(),
        base,
    };
    BLOCK_KINDS
        .set(h)
        .map_err(|_| super::InitError::AlreadyInitialized)
}

#[cfg(not(any(debug_assertions, feature = "framework")))]
pub(crate) fn install_block_kinds(_base: usize, _len: usize) -> Result<(), super::InitError> {
    Ok(())
}

#[cfg(any(debug_assertions, feature = "framework"))]
fn block_kind_table() -> (&'static mut [Kind], usize) {
    let h = BLOCK_KINDS
        .get()
        .expect("block kinds not installed (block::init 应先调 install_block_kinds)");
    // SAFETY: 见 `BlockKinds` 的 SAFETY。
    let t = unsafe { core::slice::from_raw_parts_mut(h.ptr, h.len) };
    (t, h.base)
}

/// 写下 `addr` 起 `1 << power` 字节这段的类目（`power < BLOCK_KIND_SHIFT` ⇒ 不落）。
#[cfg(any(debug_assertions, feature = "framework"))]
fn note_block_kind(addr: usize, power: usize, k: Kind) {
    if power < BLOCK_KIND_SHIFT {
        return;
    }
    let (t, base) = block_kind_table();
    let first = (addr - base) >> BLOCK_KIND_SHIFT;
    t[first..first + (1 << (power - BLOCK_KIND_SHIFT))].fill(k);
}

/// 读 `addr` 这一块的类目；`power < BLOCK_KIND_SHIFT` ⇒ `Plain`（那些块不落类目，
/// 与 [`record_block_take`] 的那一支对称）。
#[cfg(any(debug_assertions, feature = "framework"))]
fn block_kind_at(addr: usize, power: usize) -> Kind {
    if power < BLOCK_KIND_SHIFT {
        return Kind::Plain;
    }
    let (t, base) = block_kind_table();
    t[(addr - base) >> BLOCK_KIND_SHIFT]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotInitialized,
    AlreadyInitialized,
}

// ── 内部驻留 ──

struct Stats {
    frame_occupied: AtomicUsize,
    block_occupied: AtomicUsize,
    spare_occupied: AtomicUsize,
    /// 后备仓**定容**总量（`spare::init` 报一次；余量由此减去在手段数得出）。
    spare_total: AtomicUsize,
    /// 逐类在册数（下标 = `Kind::ix`；**帧与块合计一份** —— 判据是逐类净额，
    /// 与容器无关，合成一份就不必给"哪一侧"再造一条读数）。
    #[cfg(any(debug_assertions, feature = "framework"))]
    kinds: [AtomicUsize; Kind::COUNT],
}

static STATS: OnceLock<&'static Stats> = OnceLock::new();

fn stats() -> &'static Stats {
    STATS.get().expect("statistics not initialized")
}

// ── 装配 ──

pub fn init() -> Result<(), Error> {
    if STATS.get().is_some() {
        return Err(Error::AlreadyInitialized);
    }
    let s: &'static Stats = Box::leak(Box::new(Stats {
        frame_occupied: AtomicUsize::new(0),
        block_occupied: AtomicUsize::new(0),
        spare_occupied: AtomicUsize::new(0),
        spare_total: AtomicUsize::new(0),
        #[cfg(any(debug_assertions, feature = "framework"))]
        kinds: [const { AtomicUsize::new(0) }; Kind::COUNT],
    }));
    STATS.set(s).map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

// ── 写侧（分配器热路径；持分配器锁期间调用）──

/// 帧被取走：在册帧数 +1，并把该块 `2^power` 帧的类目写成**本核当前标注**。
/// 与 [`record_frame_give`] 成对（漏一次就是"净漏帧"）。
///
/// 前置：调用方持 `FrameAllocator::inner` 锁，`index` 是本次交付的块首帧索引。
pub(crate) fn record_frame_take(index: usize, power: usize) {
    let s = stats();
    s.frame_occupied.fetch_add(1, Ordering::Relaxed);
    #[cfg(any(debug_assertions, feature = "framework"))]
    {
        let k = current();
        note_frame_kind(index, 1 << power, k);
        s.kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, feature = "framework")))]
    let _ = (index, power);
}

/// 帧被归还：在册帧数 −1；类目取**这帧当初被取走时写下的**那一格（不是释放点的标注）。
///
/// 前置：调用方持 `FrameAllocator::inner` 锁，`index` 是被归还块的块首帧索引。
pub(crate) fn record_frame_give(index: usize) {
    let s = stats();
    s.frame_occupied.fetch_sub(1, Ordering::Relaxed);
    #[cfg(any(debug_assertions, feature = "framework"))]
    {
        let k = frame_kind_at(index);
        s.kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, feature = "framework")))]
    let _ = index;
}

/// 块被交付：把该块的类目写成**本核当前标注**并按类 +1。
///
/// 前置：调用方持本池锁（`pull` 路径），`addr` 是刚交付的块首地址。
pub(crate) fn record_block_take(addr: usize, power: usize) {
    #[cfg(any(debug_assertions, feature = "framework"))]
    {
        // 落不进类目表的块（`power < BLOCK_KIND_SHIFT`）：两侧都按 `Plain` 记，
        // 标注被忽略但不破坏账 —— 与 `record_block_give` 的那一支严格对称。
        let k = if power < BLOCK_KIND_SHIFT {
            Kind::Plain
        } else {
            let k = current();
            note_block_kind(addr, power, k);
            k
        };
        stats().kinds[k.ix()].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, feature = "framework")))]
    let _ = (addr, power);
}

/// 块被归还：类目取**这块当初被交付时写下的**那一格，按类 −1。
pub(crate) fn record_block_give(addr: usize, power: usize) {
    #[cfg(any(debug_assertions, feature = "framework"))]
    {
        let k = block_kind_at(addr, power);
        stats().kinds[k.ix()].fetch_sub(1, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, feature = "framework")))]
    let _ = (addr, power);
}

/// 块分配器从帧池**借走**一页（`prime` 拆块入链之前记一笔）。
pub(crate) fn record_pool_take() {
    stats().block_occupied.fetch_add(1, Ordering::Relaxed);
}

/// 块分配器**归还**一页给帧池（`drain` 归一页之后记一笔）。
pub(crate) fn record_pool_give() {
    stats().block_occupied.fetch_sub(1, Ordering::Relaxed);
}

pub(crate) fn record_spare_take(bytes: usize) {
    stats().spare_occupied.fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn record_spare_give(bytes: usize) {
    stats().spare_occupied.fetch_sub(bytes, Ordering::Relaxed);
}

/// 后备仓定容总量（`spare::init` 时报一次；之后仓不再向主堆要内存）。
pub(crate) fn record_spare_total(total: usize) {
    records(|s| s.spare_total.store(total, Ordering::Relaxed));
}

// ── 读侧（判据输入；消费者见模块头）──
//
// 读侧与它的消费者**同一个 gate**（`health/` 的三条用例只在 debug / framework 档
// 编进去）：消费者不在的档里，这些读数不存在——留一个没人读的读侧只会让"这里还有
// 条判据"变成错觉（release 档下它们曾经就是这样被编译出来又没人读的）。

#[cfg(any(debug_assertions, feature = "framework"))]
/// 帧池在手的**帧数**（非块池用途的在途帧 = 本数 − [`block_occupied`]）。
pub fn frame_occupied() -> usize {
    stats().frame_occupied.load(Ordering::Relaxed)
}

#[cfg(any(debug_assertions, feature = "framework"))]
/// 块分配器从帧池借走的页数（块池自己持着的那部分）。
pub fn block_occupied() -> usize {
    stats().block_occupied.load(Ordering::Relaxed)
}

#[cfg(any(debug_assertions, feature = "framework"))]
/// 后备仓在手的字节数。
pub fn spare_occupied() -> usize {
    stats().spare_occupied.load(Ordering::Relaxed)
}

#[cfg(any(debug_assertions, feature = "framework"))]
/// 后备仓余量（字节）= 定容总量 − 在手段数之和。
///
/// **由在手段数导出**（而不是另记一条读数）：两条账必然漂移，而"余量"本就只是
/// 差。这样仓内每拉一块/还一块，余量当场就变——`health/spare.rs` 的溢出演练
/// 断言的正是这件事（演练前后余量必须逐字节还原）。
pub fn spare_available() -> usize {
    let s = stats();
    s.spare_total
        .load(Ordering::Relaxed)
        .saturating_sub(s.spare_occupied.load(Ordering::Relaxed))
}

/// 装配完成前也能报数（仓在 init 之前就要报余量）⇒ 未装配则静默丢弃。
fn records(f: impl FnOnce(&Stats)) {
    if let Some(s) = STATS.get() {
        f(s);
    }
}

/// 逐类在册数快照（**帧与块合计**；含未标注的 [`Kind::Plain`]）。
///
/// 口径与 [`frame_occupied`] 一致：**一笔 = 一次 take**（帧侧一笔可能是 `2^power` 帧的
/// 块，块侧一笔是 `2^power` 字节的块）；类目表按"格"记是为了让归还侧问得出类别。
#[cfg(any(debug_assertions, feature = "framework"))]
pub(crate) fn kinds() -> Kinds {
    let s = stats();
    let mut out = [0usize; Kind::COUNT];
    for (i, c) in s.kinds.iter().enumerate() {
        out[i] = c.load(Ordering::Relaxed);
    }
    Kinds(out)
}

/// 逐类读数快照（`Copy`：用例取两次做差集）。
#[cfg(any(debug_assertions, feature = "framework"))]
#[derive(Clone, Copy)]
pub(crate) struct Kinds([usize; Kind::COUNT]);

#[cfg(any(debug_assertions, feature = "framework"))]
impl Kinds {
    /// 某一类的在册数。
    pub(crate) fn get(&self, k: Kind) -> usize {
        self.0[k.ix()]
    }

    /// 只列非零项（次序 = [`Kind::ALL`]）。
    pub(crate) fn nonzero(&self) -> impl Iterator<Item = (Kind, usize)> + '_ {
        Kind::ALL
            .into_iter()
            .enumerate()
            .filter_map(move |(i, k)| (self.0[i] != 0).then_some((k, self.0[i])))
    }
}

#[cfg(any(debug_assertions, feature = "framework"))]
impl core::fmt::Display for Kinds {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut any = false;
        for (k, n) in self.nonzero() {
            if any {
                f.write_str(" ")?;
            }
            any = true;
            write!(f, "{}={n}", k.name())?;
        }
        if !any {
            f.write_str("(空)")?;
        }
        Ok(())
    }
}

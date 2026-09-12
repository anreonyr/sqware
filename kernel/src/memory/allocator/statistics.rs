//! 分配子系统统一统计出口。
//!
//! 三个分配器（frame / block / spare）各自**不再保留** occupied / available 等
//! read-only 统计字段（无算法依赖）；record 钩子在持分配器锁期间同步增减本模块的
//! atomic，作为唯一权威。
//!
//! # 只留**判据输入**
//!
//! 这里只回答四个数：帧池在手的帧数、块分配器从帧池借走的页数、后备仓在手的字节数
//! 与余量。它们是 `health/` 三条用例的判据输入（"净漏帧" / "仓预算演练闭环"），
//! **没有别的东西读**——曾经有过的 `total` / `available` 水位、逐池页数、关机收尾
//! 用的 `Baseline` / `Delta` 三件套都属于审计层，随那一层一起删了：留下没人读的
//! 读数只会让"这里还有个判据"变成错觉。
//!
//! 消费者只在 debug / framework 档存在（`health/pagetable.rs` 的用例有同一个
//! gate），故本模块的读侧在 release 档下**全部**是死的——这是构造上的必然，不是
//! 遗漏：想让某个数在别的档也被读，就在那一档加一条真读它的判据。

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;

use crate::lock::OnceLock;

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
    }));
    STATS.set(s).map_err(|_| Error::AlreadyInitialized)?;
    Ok(())
}

// ── 写侧（分配器热路径；持分配器锁期间调用）──

/// 帧被取走：在册帧数 +1。与 [`record_frame_give`] 成对（漏一次就是"净漏帧"）。
pub(crate) fn record_frame_take() {
    stats().frame_occupied.fetch_add(1, Ordering::Relaxed);
}

/// 帧被归还：在册帧数 −1。
pub(crate) fn record_frame_give() {
    stats().frame_occupied.fetch_sub(1, Ordering::Relaxed);
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

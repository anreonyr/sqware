// hole — 数据过内核的管道。
//
// holeMeta 是内核侧"门洞"：单槽消息缓冲（`Vec<u8>`，capacity = mtu）+ 状态。
// 消息长度由 Push 时显式声明、Pull 时显式声明 max——长度是参数不是约定。
// 用户态 Pie<HoleMeta>（含 Weak<HoleMeta>）只持门闩，不参与数据。
//
// 数据面原语（全部非阻塞）：
// - `try_push(meta, src)`：槽空则拷 src 进 slot 并唤醒对侧 Pull。src.len() ∈ [1, mtu]。
// - `try_pull(meta, dst) -> usize`：槽非空则拷 src.len() 字节进 dst 并唤醒对侧
//   Push；返回实际长度。dst.len() < src.len() → Denied。
// - `ready(dir)`：该方向现在可用吗。
// - `wait(meta, dir, dur)`：唯一挂起入口——先探、后挂；死则报 Dead。
// - `key(meta, dir)`：该方向的等待键（命名空间 0，键不出内核）。
//
// 写/读完槽后都 wake 对侧 waiters。
//
// **锁序约定**：`slot` 是 L3 锁。envcall handler 不在持 slot 锁时调 copy_in/out
// （后者经 `space.segments` 走 Space 锁 = L2，会违反 2→4 反向嵌套）——handler
// 先把用户 VA 拷到栈/堆暂存，再调 try_push/try_pull 拷进/拷出 slot 的 Vec 存储。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use env::HoleDir;

use super::HOLE_MTU_MAX;
use super::memo::{self, Meta, ResourceId};
use crate::work::room::messenger::{self, Handoff, WaitKey};
use crate::work::unit::gate::GateError;

/// hole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

/// hole 数据面实体（Arc 持有；最后强引用 drop 时 Meta 释放）。
pub struct HoleMeta {
    state: SpinLock<HoleState>,
    /// unseal 时定；`1..=HOLE_MTU_MAX`。Push/Pull 的长度校验上限。
    pub mtu: usize,
    /// 单槽消息缓冲：`Vec<u8>` 的 capacity 恒为 mtu（创建时分配）；`len()` 既是
    /// 「消息是否在槽」也是「实际占用字节数」——`len() > 0` 即有消息，`len() == 0`
    /// 即空槽。Push 时 set_len、Pull 时 clear，零额外分配。
    slot: SpinLock<Vec<u8>>,
}

impl HoleMeta {
    pub(super) fn new(mtu: usize) -> Arc<Self> {
        let buf = Vec::with_capacity(mtu);
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            mtu,
            slot: SpinLock::new_level(Level::L3, buf),
        })
    }

    /// 存活：state == Live（Arc 仍有效由 Pie 持 Weak 保证）。
    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == HoleState::Live
    }

    /// 该方向现在可用吗：`Pull` = 槽里有消息（len > 0），`Push` = 槽空（len == 0）。
    /// 纯查询，不判存活。
    ///
    /// 与 `wait` 的挂起条件、与 `try_push`/`try_pull` 的唤醒点读同一份 `slot.len()` —
    /// 三者必须同源，否则会出现"就绪了却没人唤醒"或"唤醒后仍不满足"。
    pub(crate) fn ready(&self, dir: HoleDir) -> bool {
        let slot = self.slot.lock();
        match dir {
            HoleDir::Pull => !slot.is_empty(),
            HoleDir::Push => slot.is_empty(),
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
    }
}

// ── 等待 key（per-meta × 方向）──

/// 方向 → 等待键。命名空间恒 0（内核），故用户态构不出同键（键不出内核）；
/// 低位是方向位，两个方向的键必不相同。
///
/// 等数据的 task 等 `Pull` 键（push 写完槽后唤醒），等空位的 task 等 `Push` 键
/// （pull 取完槽后唤醒）。
pub(crate) fn key(meta: &HoleMeta, dir: HoleDir) -> WaitKey {
    let raw = meta as *const _ as usize;
    match dir {
        // 显式把 `raw | 1` 拆成两个语句、命名中间变量：size 优化下 `| 1` 不会再
        // 合并进 `WaitKey::compose` 的 mask 计算路径（见 §13.10 A 待办方向 B）。
        HoleDir::Pull => {
            let with_low_bit = raw | 1;
            WaitKey::compose(0, with_low_bit)
        }
        // 显式把 raw 拆出来给变量：避免 size 优化把 `| 1` 折叠到 compose 的 mask
        // 参数里（见 §13.10 A 待办方向 B）。
        HoleDir::Push => WaitKey::compose(0, raw),
    }
}

// ── 数据面原语（非阻塞）──

/// 非阻塞 push：槽空则拷 `src` 进 slot 并 wake 等读的；槽满返 Busy。
///
/// 前置：`src.len() ∈ [1, mtu]`。envcall 入口已校验 `len <= mtu`，此处再 defend。
/// 调用方须在持 `src` 时不持 slot 锁（slot = L3，Space.segments = L2；持 L3
/// 调 L2 锁为 4→2 反向嵌套）。
pub(crate) fn try_push(meta: &HoleMeta, src: &[u8]) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let len = src.len();
    if len == 0 || len > meta.mtu {
        return Err(GateError::Denied);
    }
    let mut slot = meta.slot.lock();
    if !slot.is_empty() {
        return Err(GateError::Busy);
    }
    // SAFETY: capacity == mtu >= len；src 含 len 字节。
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), slot.as_mut_ptr(), len);
        slot.set_len(len);
    }
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Pull));
    Ok(())
}

/// 非阻塞 pull：槽非空则拷 `src.len()` 字节进 `dst` 并 wake 等写的；槽空返 Busy。
/// 返实际长度。`dst.len() < src.len()` 返 Denied（buf 装不下）。
///
/// 锁序同 try_push：调用方持 `dst` 时不持 slot 锁。
pub(crate) fn try_pull(meta: &HoleMeta, dst: &mut [u8]) -> Result<usize, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let mut slot = meta.slot.lock();
    let len = slot.len();
    if len == 0 {
        return Err(GateError::Busy);
    }
    if dst.len() < len {
        return Err(GateError::Denied);
    }
    // SAFETY: dst 含至少 len 字节；slot 含 len 字节已 set_len。
    unsafe {
        core::ptr::copy_nonoverlapping(slot.as_ptr(), dst.as_mut_ptr(), len);
    }
    slot.clear();
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Push));
    Ok(len)
}

// ── 挂起（唯一入口）──

/// 等待结果（核心语义，不含 ABI 映射）。
pub(crate) enum Waited {
    /// 未挂起：`true` = 该方向现在就绪。
    Resume(bool),
    /// 已挂起：`Some(pa)` = 切到该帧；`None` = 本核无后备（调用方取活）。
    Parked(Option<usize>),
}

/// 等某方向就绪：死 → `Err(Dead)`；就绪或 `dur == 0` → 不挂起；否则挂起。
///
/// 前置：调用者在任务上下文，且**不持任何 L3 锁**（`wait_sites` 是 L3，3→3 禁止）。
///
/// 「先探」在此处不可省：对侧可能已经写入并正等我们取，此时若我们 park 在"等写入"
/// 上就永远等不到下一次唤醒。先探与登记之间的窗口由 messenger 的 pend 双检封住
/// （窗口内的 wake 置 pend，登记时被消费 ⇒ 不挂起）。
pub(crate) fn wait(meta: &HoleMeta, dir: HoleDir, dur: Duration) -> Result<Waited, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    if meta.ready(dir) {
        return Ok(Waited::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Waited::Resume(false));
    }
    Ok(match messenger::wait(key(meta, dir), dur) {
        // 窗口内 wake 已至（未挂起）：以当前状态为准。
        Handoff::Resume => Waited::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Waited::Parked(Some(pa)),
        Handoff::Idle => Waited::Parked(None),
    })
}

// ── 封印 ──

/// 封印 hole：置死 → 唤醒两个方向的**全部**等待者 → memo 移除。
///
/// 唤醒必须在 `memo::remove` **之前**：最后一份强引用就在 memo 表里，remove 在
/// L3 锁内 drop，此时再 wake（sites 亦 L3）构成 3→3 嵌套。`wake` 只弹队首，故
/// 循环到空；被唤醒者重判 alive 得 Dead，不会挂死。
pub(crate) fn seal(meta: &HoleMeta, id: ResourceId) {
    *meta.state.lock() = HoleState::Dead;
    while messenger::wake(key(meta, HoleDir::Pull)) {}
    while messenger::wake(key(meta, HoleDir::Push)) {}
    memo::remove(id);
}

// ── 创建 ──

/// 解封 hole 的资源实体：建 Meta + 注册 memo。**不落 pies**——建门闩与落
/// `task.pies` 由 envcall 编排（gate::new_pie + pies.push）。返 `(Arc, ResourceId)`：
/// `ResourceId` 供 gate::new_pie 第一参，`Arc` 供 Weak<HoleMeta>。
///
/// `mtu ∈ [1, HOLE_MTU_MAX]`——envcall 入口已校验，此处 defend。
pub(crate) fn meta(mtu: usize) -> Result<(Arc<HoleMeta>, ResourceId), GateError> {
    if mtu == 0 || mtu > HOLE_MTU_MAX {
        return Err(GateError::Denied);
    }
    let arc = HoleMeta::new(mtu);
    let id = memo::alloc_id();
    memo::insert(id, Meta::Hole(arc.clone()));
    Ok((arc, id))
}

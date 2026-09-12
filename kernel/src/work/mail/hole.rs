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

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use env::HoleDir;

use super::HOLE_MTU_MAX;
use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::GateError;
use crate::work::unit::life::Life;

/// Hole 的全局身份（自 1 递增、永不复用）——**等待键的身份**（见 [`key`]）。
///
/// 键取它而不取 `HoleMeta` 的堆地址：站点表的站点从不回收，而地址会被
/// 分配器回收再利用——死孔留下的陈旧 pend 会被落在同一地址的新孔继承。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoleId(pub usize);

fn alloc_id() -> HoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    HoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// hole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

/// 单槽：消息 + 它的来源。
///
/// `from` = 推者的 task id，**内核在 Push 时盖章**（syscall 上下文不可伪造）。
/// 收方 Pull 时一并拿到——身份不再需要从报文里猜。
struct Slot {
    /// capacity 恒为 mtu（创建时分配）；`len() > 0` 即有消息。
    buf: Vec<u8>,
    /// 当前槽里这条消息的发送者；空槽时无意义。
    from: usize,
}

/// hole 数据面实体（Arc 持有；最后强引用 drop 时 Meta 释放）。
pub struct HoleMeta {
    state: SpinLock<HoleState>,
    /// 本 hole 的全局资源 id——**等待键的身份**（单调分配、永不复用；见 [`key`]）。
    id: HoleId,
    /// 本 hole 的**存活单元**（两个方向的等待键都指它）：强持有者是本 Meta ⇒
    /// 最后一份门闩消失时键自然判死，站点随之可删（A2）。见 [`Life`]。
    life: Arc<Life>,
    /// unseal 时定；`1..=HOLE_MTU_MAX`。Push/Pull 的长度校验上限。
    pub mtu: usize,
    /// 单槽消息：`len()` 既是「消息是否在槽」也是「实际占用字节数」——Push 时
    /// set_len、Pull 时 clear，零额外分配。`from` 与消息同锁同写。
    slot: SpinLock<Slot>,
    /// 开辟者：`UnsealHole` 时的任务 id（构造期定型，无 setter）。0 = 内核自建。
    ///
    /// 与门闩的 `sire` 分工：`sire` = **这枚门闩**从哪来（派生边）；`owner` =
    /// **这扇门**谁开的（任意副本共享同一事实）。
    owner: usize,
}

impl HoleMeta {
    pub(super) fn new(mtu: usize, id: HoleId, owner: usize) -> Arc<Self> {
        let slot = Slot {
            buf: Vec::with_capacity(mtu),
            from: 0,
        };
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            id,
            life: Life::new(),
            mtu,
            slot: SpinLock::new_level(Level::L3, slot),
            owner,
        })
    }

    /// 本 hole 的存活单元（弱引用）——`WakeKey::Hole{hole: id, dir}` 的寿命来源。
    ///
    /// 一律 `Arc::downgrade(&self.life)`（一个弱计数 +1），**不是**每次等待一次的
    /// 搜索：`Weak` 就在 Meta 里，取值是纯函数。
    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    /// 资源开辟者（见字段 `owner`）。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// 存活：state == Live。
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
            HoleDir::Pull => !slot.buf.is_empty(),
            HoleDir::Push => slot.buf.is_empty(),
        }
    }
}

impl Drop for HoleMeta {
    /// 最后一份强引用消失：置死 + 唤醒两方向**全部**等待者。
    ///
    /// 调用方义务：**在锁外** drop 门闩——`messenger::wipe` 是 L3，在 `Task.pies`
    /// 锁内 drop 即 3→3 嵌套。被唤醒者重解析 token 时会发现门闩已不在表里
    /// （`Denied`），不会挂死。
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
        // 站点当场删掉（不留墓碑）：本函数是这两个键的**最后一次**入口——`wipe` 之后
        // 本 Meta 就归零，键随即判死，此后再没有任何入口会碰这两个键。
        messenger::wipe(key(self, HoleDir::Pull));
        messenger::wipe(key(self, HoleDir::Push));
    }
}

// ── 等待 key（per-meta × 方向）──

/// 方向 → 等待键。命名空间恒 0（内核），故用户态构不出同键（键不出内核）；
/// 低位是方向位，两个方向的键必不相同。
///
/// 等数据的 task 等 `Pull` 键（push 写完槽后唤醒），等空位的 task 等 `Push` 键
/// （pull 取完槽后唤醒）。
///
/// **键取 `HoleId` 而不是 `HoleMeta` 的堆地址**：站点表的站点从不回收，
/// 而 wake 找不到等待者时置的「唤醒闩（pend）」会一直留着；地址会被分配器回收再
/// 利用——死 hole 的陈旧 pend 会被落在同一地址的新 hole 继承，于是一次无关的
/// `wait` 立即返回「已唤醒」。id 单调分配、永不复用，无此问题。
///
/// 方向是键的一个字段，不占位：同一 hole 两方向不撞键，也不会与另一个 hole 的键
/// 相撞——**不需要位打包**。旧版把方向压进最低位，还因此要把 `| 1` 拆句去躲
/// size 优化把它折叠进 mask；枚举下这层防御连同理由一起消失。
pub(crate) fn key(meta: &HoleMeta, dir: HoleDir) -> WakeKey {
    WakeKey::Hole {
        hole: meta.id.0,
        dir,
    }
}

// ── 数据面原语（非阻塞）──

/// 非阻塞 push：槽空则拷 `src` 进 slot 并 wake 等读的；槽满返 Busy。
///
/// `from` = 推者 task id（内核在 envcall 入口盖章）——与消息**同锁同写**，收方
/// Pull 时一并取回。
///
/// 前置：`src.len() ∈ [1, mtu]`。envcall 入口已校验 `len <= mtu`，此处再 defend。
/// 调用方须在持 `src` 时不持 slot 锁（slot = L3，Space.segments = L2；持 L3
/// 调 L2 锁为 4→2 反向嵌套）。
pub(crate) fn try_push(meta: &HoleMeta, src: &[u8], from: usize) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let len = src.len();
    if len == 0 || len > meta.mtu {
        return Err(GateError::Denied);
    }
    let mut slot = meta.slot.lock();
    if !slot.buf.is_empty() {
        return Err(GateError::Busy);
    }
    // SAFETY: capacity == mtu >= len；src 含 len 字节。
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), slot.buf.as_mut_ptr(), len);
        slot.buf.set_len(len);
    }
    slot.from = from;
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Pull), &meta.life());
    Ok(())
}

/// 非阻塞 pull：槽非空则拷 `src.len()` 字节进 `dst` 并 wake 等写的；槽空返 Busy。
/// 返 `(实际长度, 发送者 task id)`。`dst.len() < src.len()` 返 Denied（buf 装不下）。
///
/// 锁序同 try_push：调用方持 `dst` 时不持 slot 锁。
pub(crate) fn try_pull(meta: &HoleMeta, dst: &mut [u8]) -> Result<(usize, usize), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let mut slot = meta.slot.lock();
    let len = slot.buf.len();
    if len == 0 {
        return Err(GateError::Busy);
    }
    if dst.len() < len {
        return Err(GateError::Denied);
    }
    // SAFETY: dst 含至少 len 字节；slot 含 len 字节已 set_len。
    unsafe {
        core::ptr::copy_nonoverlapping(slot.buf.as_ptr(), dst.as_mut_ptr(), len);
    }
    let from = slot.from;
    slot.buf.clear();
    drop(slot);
    let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
    Ok((len, from))
}

// ── 挂起（唯一入口）──

/// 等某方向就绪：死 → `Err(Dead)`；就绪或 `dur == 0` → 不挂起；否则挂起。
///
/// 前置：调用者在任务上下文，且**不持任何 L3 锁**（站点表是 L3，3→3 禁止）。
///
/// 「先探」在此处不可省：对侧可能已经写入并正等我们取，此时若我们 park 在"等写入"
/// 上就永远等不到下一次唤醒。先探与登记之间的窗口由 messenger 的 pend 双检封住
/// （窗口内的 wake 置 pend，登记时被消费 ⇒ 不挂起）。
pub(crate) fn wait(
    meta: &HoleMeta,
    dir: HoleDir,
    dur: Duration,
) -> Result<Handoff<bool>, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    if meta.ready(dir) {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    // `meta.life()` 的临时量**直接移进**等待机（站点是它唯一的持有者）：这一帧里
    // 不留副本，故挂起跨过的调用链上没有任何引用需要析构。
    Ok(match messenger::wait(key(meta, dir), meta.life(), dur) {
        // 窗口内 wake 已至（未挂起）：以当前状态为准。
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

// ── 封印 ──

/// 封印 hole：置死 + 唤醒两方向**全部**等待者。
///
/// **不回收内存**——资源寿命由引用计数决定：最后一份门闩消失时 `Drop` 接管回收
/// （它也会唤醒，此处唤醒是为了让封印**立即**对等待者生效）。
///
/// 调用方不持 L3 锁（`wake` 是 L3）。
pub(crate) fn seal(meta: &HoleMeta) {
    *meta.state.lock() = HoleState::Dead;
    messenger::wipe(key(meta, HoleDir::Pull));
    messenger::wipe(key(meta, HoleDir::Push));
}

// ── 创建 ──

/// 解封 hole 的资源实体：建 Meta。**不落 pies**——建门闩与落 `task.pies` 由
/// envcall 编排（gate::new_pie + pies.push）。返 `Arc`：它既是资源实体，也是
/// 门闩持有的**唯一强引用**（资源寿命 = 能力寿命）。
///
/// `mtu ∈ [1, HOLE_MTU_MAX]`——**唯一校验点**：`UnsealHole` 在 envcall 入口把 mtu
/// 直交这里，没有第二道。
/// `owner` = 开辟者任务 id（envcall 入口传当前任务）。
pub(crate) fn meta(mtu: usize, owner: usize) -> Result<Arc<HoleMeta>, GateError> {
    if mtu == 0 || mtu > HOLE_MTU_MAX {
        return Err(GateError::Denied);
    }
    // 先分配 id 再建 Meta：id 同时是等待键的身份（见 `key`），必须随 Meta 定型。
    Ok(HoleMeta::new(mtu, alloc_id(), owner))
}

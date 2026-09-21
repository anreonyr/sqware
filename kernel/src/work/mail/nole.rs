// Nole — 无载荷的权柄载体。
//
// # 名字
//
// `Nole` = **no + -ole**：与 `Hole`（洞）、`Pole`（极）同族，而它说的是"**没有**"。
// 三个名字各说一个字的形状：
//
//     Hole  有槽（消息穿孔）   —— 有数据面
//     Pole  有页（页视图）     —— 有数据面
//     Nole  **什么都没有**     —— 无数据面
//
// 名字即定义：`Nole` 承诺数据面为空，故它不可能被当成资源来使唤。
//
// 与 Hole / Pole 并列的第三种数据面**类型**，也是三者中唯一**没有数据面**的：
// 它不携带消息（那是 Hole），也不借映页（那是 Pole）。全部内容就是"存在"这个事实
// ——故它的 meta 比另两者少掉整个载荷部分，只剩身份、存活与一位门铃。
//
// 用途：**无载荷通信的载体**——即"有事/没事"，与"你对某份资源能做什么"无关。
// 唯一消费者是**门铃**（`Bell`，`crates/runtime/src/core/bell.rs`）：把一枚 Nole 当
// "内核一件事实的出口"用——等它响、应它、由内核响它。
//
// 听者面（`id` / `life` / 一位 `ring`）由此而来：门铃有听者，故有等待键；响要能被
// "响在没人听的那一刻"记住，故有一位。它**不带来第二种类型**——门铃仍是一枚 Nole，
// "怎么用"封装在 runtime 层。
//
// **曾经的第一位消费者是"建域权"（`UnitCall::Build` 收一枚 Nole），已删**：那枚门
// 零成本可自铸（`UnsealNole` 只查 S 态），于是"门一 ∧ S 态 ≡ S 态"——它不是门。
// 建域权就是 S 态（见 `env::fid` 该 variant）。删它正是为了不让一枚铃顺带成为建域资格。
//
// 为什么不是 Permission 的一位：位说"对这份资源能做什么"，与资源同轴；而"有事/没事"
// 不属于任何资源。做成位会让**任何**资源顺带携带它（`FETCH|STORE|BUILD` 这种掩码
// 一旦能出现，"这是不是那枚"就再也答不出来）。类型是身份，位不是。
//
// 为什么不是"没数据的 Hole"：那样权威长得跟普通资源一样，内核又只能靠标记去认它
// ——回到位那条路。Nole **因为空，所以不可能被当成资源来使唤**。
//
// 边界（定义式）：Nole 只承载**无载荷**的信号。带状态的许可（配额"最多 N 个"、
// 设备能力"哪个窗口"）不该是 Nole——那种要另立 meta，因为"多少/哪个"是数据面的活。
// **唯一允许的状态是门铃那一位**：它答的是"有事/没事"，没有"多少/哪个"可言，
// 故它不是数据面。

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::unit::gate::GateError;
use crate::work::unit::life::Life;

/// Nole 的全局身份（自 1 递增、永不复用）——**听者键的身份**（见 [`key`]）。
///
/// 键取它而不取 `NoleMeta` 的堆地址：站点在资源死亡时由 `prune` 删除（不留墓碑），
/// 而地址会被分配器回收再利用——地址复用会让两枚铃的身份相等。
///
/// 与 `HoleId` **各起一份计数器**：键不同族（`WakeKey::Nole` / `WakeKey::Hole`），
/// 数值相撞也不撞键；共用一份只会把两个模块的寿命绑在一起。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NoleId(pub usize);

fn alloc_id() -> NoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    NoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// Nole 状态。与 Hole/Pole 同词：`Seal` 之后恒 `Dead`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoleState {
    Live,
    Dead,
}

/// Nole 数据面实体（Arc 持有；**无载荷**——没有 mtu、没有槽、没有映射表）。
pub struct NoleMeta {
    state: SpinLock<NoleState>,
    /// 本 Nole 的全局资源 id——**听者键的身份**（单调分配、永不复用；见 [`key`]）。
    id: NoleId,
    /// 本 Nole 的**存活单元**（听者键指它）：强持有者是本 Meta ⇒ 最后一份门闩消失
    /// 时键自然判死，站点随之可删。见 [`Life`]。
    life: Arc<Life>,
    /// 有待取之事：铃响过了、还没被应。
    ///
    /// 它是"响"的**记忆**——响可以落在没人听的那一刻（听者正忙），若响只等于
    /// "唤醒站点"，那一枚就丢了。对中断那道门铃，这一位同时就是内核侧闸门的账
    /// （响着 ⇒ 本 hart 的 SEIE 关着）。
    ring: SpinLock<bool>,
    /// 开辟者：`UnsealNole` 时的任务 id（构造期定型，无 setter）。0 = 内核自建。
    /// 语义同 `HoleMeta::owner`：`vestor` 管门闩的来历，`owner` 管资源的来历。
    owner: usize,
}

impl NoleMeta {
    /// 造一枚 Nole。**无参数**——没有大小、没有对齐、没有上限可校验，这正是它
    /// 与 `hole::meta(owner, mark)` / `pole::allocate(bytes, owner)` 的区别。
    /// （另外三个字段都是"听者面"，构造期定型，没有 setter。）
    pub(crate) fn new(owner: usize) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, NoleState::Live),
            id: alloc_id(),
            life: Life::new(),
            ring: SpinLock::new_level(Level::L3, false),
            owner,
        })
    }

    /// 本 Nole 的存活单元（弱引用）——`WakeKey::Nole{id}` 的寿命来源。
    /// 与 `HoleMeta::life` 同款：`Weak` 就在 Meta 里，取值是纯函数。
    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    /// 资源开辟者（见字段 `owner`）。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// 本 Nole 的全局身份（组的格子按它认成员——格子记资源身份，不记句柄）。
    pub(crate) fn id(&self) -> NoleId {
        self.id
    }

    /// 资源可用（已封印 → false）。
    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), NoleState::Live)
    }

    /// 铃现在响着吗（有待取之事）。纯查询，不判存活。
    ///
    /// 与 `wait` 的挂起条件、与 `ring` 的 `Busy` 判据、与 `hush` 的清位读的是
    /// **同一格**——三者必须同源，否则会出现"响了却没人唤醒"或"唤醒后仍不满足"。
    pub(crate) fn ready(&self) -> bool {
        *self.ring.lock()
    }
}

impl Drop for NoleMeta {
    /// 最后一份强引用消失：置死 + 唤醒**全部**听者。
    ///
    /// 调用方义务：**在锁外** drop 门闩——`messenger::wipe` 是 L3，在 `Task.pies`
    /// 锁内 drop 即 3→3 嵌套。被唤醒者重解析 token 时会发现门闩已不在表里
    /// （`Denied`），不会挂死。
    fn drop(&mut self) {
        *self.state.lock() = NoleState::Dead;
        messenger::wipe(key(self));
    }
}

// ── 听者键（per-meta）──

/// 听者 → 等待键。**没有方向字段**：门铃只有一条方向（有事/没事），
/// 这也是 [`wait`] 不收 `dir` 的理由（对照 `hole::key(meta, dir)`）。
///
/// **键取 `NoleId` 而不是 `NoleMeta` 的堆地址**：理由同 `hole::key`——站点由
/// `prune` 在资源死亡时删除，而地址会被复用，身份将不再唯一。
pub(crate) fn key(meta: &NoleMeta) -> WakeKey {
    WakeKey::Nole { id: meta.id.0 }
}

// ── 听者面原语（非阻塞）──

/// 非阻塞响：置位 + 唤醒听者；**已响返 `Busy`**。
///
/// `Busy` 不是错误，是"先把上一件取走"：对中断那道门铃，它正是内核据此**关本 hart
/// 闸门**的信号（`trap/mod.rs` 的外部中断分支），也是"多 hart 同响合成一位"的落点。
///
/// **没有 `from` 参数**——响没有来源（对照 `try_push(meta, src, from)`：那里的
/// `from` 是"谁推的"，要交给收方）。
///
/// 调用方不持 L3 锁（`wake` 是 L3）。
pub(crate) fn ring(meta: &NoleMeta) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    {
        let mut ring = meta.ring.lock();
        if *ring {
            return Err(GateError::Busy);
        }
        *ring = true;
    }
    let _ = messenger::wake(key(meta), &meta.life());
    Ok(())
}

/// 非阻塞应铃：清位。**未响返 `Busy`**（同 `try_take` 的"没东西可取"）。
///
/// **不唤醒任何人**——没人等"铃不响"（对照 `try_take` 取完要唤醒等写的）。
/// 不看 `alive`：清位是收场动作，拆铃之后剩的那一位仍要有人来清。
pub(crate) fn hush(meta: &NoleMeta) -> Result<(), GateError> {
    let mut ring = meta.ring.lock();
    if !*ring {
        return Err(GateError::Busy);
    }
    *ring = false;
    Ok(())
}

// ── 挂起（唯一入口）──

/// 等铃：死 → `Err(Dead)`；响着或 `dur == 0` → 不挂起；否则挂起。
///
/// **不收 `dir`**：方向只有一条。前置与 `hole::wait` 同：调用者在任务上下文，
/// 且**不持任何 L3 锁**（站点表是 L3，3→3 禁止）。
///
/// 「先探」同样不可省：响可能已经置起而无人消费，此时若我们 park 在"等响"上就
/// 永远等不到下一次唤醒。先探与登记之间的窗口由 messenger 的 pend 双检封住
/// （窗口内的 wake 置 pend，登记时被消费 ⇒ 不挂起）。
pub(crate) fn wait(meta: &NoleMeta, dur: Duration) -> Result<Handoff<bool>, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    if meta.ready() {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    // `meta.life()` 的临时量**直接移进**等待机（站点是它唯一的持有者）：这一帧里
    // 不留副本，故挂起跨过的调用链上没有任何引用需要析构。
    Ok(match messenger::wait(key(meta), meta.life(), dur)? {
        // 窗口内 wake 已至（未挂起）：以当前状态为准——陈旧信标不得冒充铃响。
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready()),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

// ── 封印 ──

/// 封印 Nole：置死 + 唤醒听者。**不回收内存**——资源寿命由引用计数决定（同 Hole/Pole）。
///
/// 只有**一条键**要 wipe：门铃没有第二个方向（Hole 的 `seal` 要 wipe 两个方向）。
/// 已置起的 `ring` 位不在这里清——那是 `hush` 的事，且清不清都不影响"死铃不再响"。
pub(crate) fn seal(meta: &NoleMeta) {
    *meta.state.lock() = NoleState::Dead;
    messenger::wipe(key(meta));
}

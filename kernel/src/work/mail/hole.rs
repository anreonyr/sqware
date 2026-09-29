use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};

use env::{HoleDir, TaskId};

use crate::work::room::messenger::{self, Handoff, WakeKey};
use crate::work::room::scheduler::core::muster;
use crate::work::unit::life::Life;
use crate::work::unit::space::Space;
use env::MailFail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoleId(pub usize);

fn alloc_id() -> HoleId {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    HoleId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleState {
    Live,
    Dead,
}

/// 孔上**待取之事**：同一时刻最多一件——一只递出的手，或一个已响的位。
///
/// 手与位写在同一个枚举里（不是两格），故"又有手又有位"写不出来。
///
/// `Taking` 是复制那一瞬的公示：手仍在这一格，但正被一位取用者搬。取用中不算可取
/// （别人见它答 `Busy`），故一只手只会被交付一次。
enum Pending {
    Idle,
    Hand(Hand),
    Taking(Hand),
    Rung,
}

/// 递出的那只手：发送方那段内存在孔上的登记。**不含字节**——字节仍躺在发送方那儿，
/// 取走的那一刻复制**一次**。
///
/// `space` 持**弱**引用：发送方退场时这一格随之作废，且不拖住它那个空间的回收——
/// "消息随人走"由此成立，不需要另一本账。
struct Hand {
    from: TaskId,
    space: Weak<Space>,
    va: usize,
    len: usize,
}

impl Hand {
    fn own(&self) -> Option<(Arc<Space>, usize, usize)> {
        Some((self.space.upgrade()?, self.va, self.len))
    }
}

pub struct HoleMeta {
    state: SpinLock<HoleState>,
    id: HoleId,
    life: Arc<Life>,
    owner: TaskId,
    pending: SpinLock<Pending>,
}

impl HoleMeta {
    pub(super) fn new(id: HoleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            id,
            life: Life::new(),
            owner,
            pending: SpinLock::new_level(Level::L3, Pending::Idle),
        })
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn id(&self) -> HoleId {
        self.id
    }

    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == HoleState::Live
    }

    /// 就绪：`Pull` = 有可取之事（**一只手，或一个已响的位**）；`Push` = 孔空着（可递）。
    /// `Taking` 不算可取——那一只手正被搬。
    pub(crate) fn ready(&self, dir: HoleDir) -> bool {
        let pending = self.pending.lock();
        match dir {
            HoleDir::Pull => matches!(*pending, Pending::Hand(_) | Pending::Rung),
            HoleDir::Push => matches!(*pending, Pending::Idle),
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
        messenger::wipe(key(self, HoleDir::Pull));
        messenger::wipe(key(self, HoleDir::Push));
    }
}

pub(crate) fn key(meta: &HoleMeta, dir: HoleDir) -> WakeKey {
    WakeKey::Hole {
        hole: meta.id.0,
        dir,
    }
}

/// 这一枚孔的**主人**还在吗（`muster` 那一格升得上来 = 还在）。
///
/// 用于 [`give`]：主人走了 ⇒ 这只手没人来取。判据与 [`usable`](super::super::unit::gate)
/// 查"交出去那一格"用的是同一张表，故不新增账。
fn owner_alive(meta: &HoleMeta) -> bool {
    muster(meta.owner()).and_then(|w| w.upgrade()).is_some()
}

/// 递出一只手：孔空着就登记并唤醒等取的人。
///
/// **不搬字节、不分配**：登记的就是发送方那一段（`va`/`len`），复制由取的一方做（一处）。
/// 孔上已有手／正被取用／位已响 ⇒ `Busy`。
///
/// **主人不在 ⇒ 不收**（`Dead`）：这一枚孔的主人就是**等着读它的那一位**（本仓的规矩：谁读
/// 谁铸——服务入口由服务铸、回信孔由客人铸）。它走了，这只手就没人来取；旧语义里"寄放"
/// 把这件事吸收了（推了就走），只手不载字节之后它就得在**这里**回答。查的是主人那一格，
/// 一次表查询；`Taking` 里那只手不算（有取用者在场）。
pub(crate) fn give(
    meta: &HoleMeta,
    space: &Arc<Space>,
    va: usize,
    len: usize,
    from: TaskId,
) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if len == 0 {
        return Err(MailFail::Denied);
    }
    if !owner_alive(meta) {
        return Err(MailFail::Dead);
    }
    let mut pending = meta.pending.lock();
    if !matches!(*pending, Pending::Idle) {
        return Err(MailFail::Busy);
    }
    *pending = Pending::Hand(Hand {
        from,
        space: Arc::downgrade(space),
        va,
        len,
    });
    drop(pending);
    let _ = messenger::wake(key(meta, HoleDir::Pull), &meta.life());
    Ok(())
}

/// 认下要取的那只手，把它置成 `Taking`；`Ok` 之后用 [`source`] 取复制所需的四格。
///
/// **复制在孔锁之外做**：`Space` 的锁是 `Level::Space`（2）、孔这一格是 `Level::L3`（4），
/// 持孔锁再取空间锁是倒序（debug 档 lockdep 当场报），而复制每页都要过一遍 `translate`。
/// 故这里只把"正被取用"公示出去，复制成由 [`taken`] 收尾、败由 [`back`] 把手放回。
///
/// 发送方那个空间已经回收（弱引用升不上来）⇒ 这一格**作废**（消息随人走）并答 `Gone`，
/// 孔回到可用——不作废的话，一位退场的发送方会把这条孔永久堵死。
pub(crate) fn take(meta: &HoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut pending = meta.pending.lock();
    let cur = core::mem::replace(&mut *pending, Pending::Idle);
    match cur {
        Pending::Hand(hand) => match hand.own() {
            Some(_) => {
                *pending = Pending::Taking(hand);
                Ok(())
            }
            None => {
                drop(pending);
                let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
                Err(MailFail::Gone)
            }
        },
        other => {
            *pending = other;
            Err(MailFail::Busy)
        }
    }
}

/// 取用中那只手的复制源：`(发送者, 它那段空间, 起点, 长度)`。
///
/// 只在 [`take`] 与 [`taken`]／[`back`] 之间非 `None`——那一段窗口里只有本取用者进得来。
pub(crate) fn source(meta: &HoleMeta) -> Option<(TaskId, Arc<Space>, usize, usize)> {
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Taking(hand) => hand
            .own()
            .map(|(space, va, len)| (hand.from, space, va, len)),
        _ => None,
    }
}

/// 复制成功：孔回到空闲，唤醒等递的那一方。
pub(crate) fn taken(meta: &HoleMeta) {
    let mut pending = meta.pending.lock();
    if matches!(*pending, Pending::Taking(_)) {
        *pending = Pending::Idle;
    }
    drop(pending);
    let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
}

/// 复制没成：**手原样放回**。
///
/// 这一条与 `MailCall::Pull` 的四条不消费路径是同一条口径：装不下、缓冲不可写、
/// 对面那段没了——**都不替调用方丢东西**（丢一条消息不可逆）。
pub(crate) fn back(meta: &HoleMeta) {
    let mut pending = meta.pending.lock();
    let cur = core::mem::replace(&mut *pending, Pending::Idle);
    match cur {
        Pending::Taking(hand) => *pending = Pending::Hand(hand),
        other => *pending = other,
    }
}

/// **撤手**：把**我自己**伸出、还没被取走的那只手收回来。
///
/// 幂等性不是它的义务：手上没有我的东西 ⇒ `Busy`。**正被取用中**（`Taking`）也答 `Busy`——
/// 那一刻复制在另一颗 hart 上做，撤了它会读到一半；发送方该再复核一次（复制成了孔就空了）。
///
/// 仓里的用法只有一处：`HolePie::push` 的期限到了（对面没人来取）——把"永久挂"降成
/// "可诊断的 `Busy`"。**没有它，借条就悬着**：递出的那段内存在调用方手里已经失效。
pub(crate) fn withdraw(meta: &HoleMeta, from: TaskId) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let mut pending = meta.pending.lock();
    match &*pending {
        Pending::Hand(hand) if hand.from == from => {
            *pending = Pending::Idle;
            drop(pending);
            let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
            Ok(())
        }
        _ => Err(MailFail::Busy),
    }
}

/// 只看一眼那只手：`(长度, 发送者)`。**不动状态**（取用中的那只也照报）。
pub(crate) fn peek(meta: &HoleMeta) -> Result<(usize, TaskId), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Hand(hand) | Pending::Taking(hand) => Ok((hand.len, hand.from)),
        _ => Err(MailFail::Busy),
    }
}

/// 置位：孔空 ⇒ 置位并唤醒等取的人；已置 ⇒ `Busy`（**不累积**，与门铃同一口径）。
///
/// 与门铃 `ring` 同名同形：**位不占字节**，故这一路零复制、零分配、也不阻塞发送方。
pub(crate) fn ring(meta: &HoleMeta) -> Result<(), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    {
        let mut pending = meta.pending.lock();
        if !matches!(*pending, Pending::Idle) {
            return Err(MailFail::Busy);
        }
        *pending = Pending::Rung;
    }
    let _ = messenger::wake(key(meta, HoleDir::Pull), &meta.life());
    Ok(())
}

/// 清位。**不唤醒任何人**：没人等"铃不响"（与门铃 `hush` 同一句）。
pub(crate) fn hush(meta: &HoleMeta) -> Result<(), MailFail> {
    let mut pending = meta.pending.lock();
    if !matches!(*pending, Pending::Rung) {
        return Err(MailFail::Busy);
    }
    *pending = Pending::Idle;
    Ok(())
}

pub(crate) fn wait(
    meta: &HoleMeta,
    dir: HoleDir,
    dur: Duration,
) -> Result<Handoff<bool>, MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    if meta.ready(dir) {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta, dir), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

pub(crate) fn seal(meta: &HoleMeta) {
    *meta.state.lock() = HoleState::Dead;
    messenger::wipe(key(meta, HoleDir::Pull));
    messenger::wipe(key(meta, HoleDir::Push));
}

pub(crate) fn meta(owner: TaskId) -> Arc<HoleMeta> {
    HoleMeta::new(alloc_id(), owner)
}

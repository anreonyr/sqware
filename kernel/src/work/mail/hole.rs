use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;

use crate::lock::{Level, SpinLock};
use crate::runtime::chrono::clock;

use env::{HoleDir, TaskId};

use crate::work::room::messenger::{self, Handoff, WakeKey};
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

/// 孔上**待取之事**：一位写者排下的**那一列手**，或一个已响的位。
///
/// 手与位写在同一个枚举里（不是两格），故"又有手又有位"写不出来。
///
/// **`Queue` 是一条有界 FIFO**（[`QUEUE_CAP`] 只手）：写者可以连推几只、不必等对侧取走一只；
/// 读者按先进先出逐只取。`Idle` 与"队列空"是同一件事——空了就回到 `Idle`，不留空壳。
enum Pending {
    Idle,
    Queue(Queue),
    Rung,
}

/// 排着的那一列手 ＋ "队头正被取用"那一位。
///
/// **队头正被取用**是复制那一瞬的公示：手仍在本列里，但正被一位取用者搬。取用中不算可取
/// （别人见它答 `Busy`），故一只手只会被交付一次。它必须活在孔锁里——复制在孔锁**之外**做
/// （锁序 `Space`(L2) < 孔(L4)，见 [`take`] 那一节）。
struct Queue {
    /// 排着的那些手（先进先出）。**懒分配**：没排过队的孔一点内核堆都不占。
    hands: VecDeque<Hand>,
    /// 队头正被取用（复制中）。
    taking: bool,
}

/// 一只孔上最多排几只（一处常量）。**取 1 就等于改这一形之前的行为**（单槽）。
const QUEUE_CAP: usize = 4;

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
    /// **这一只**落手那一刻（ticks）：队列里排着好几只时，"谁压了多久"由它各自说
    /// （孔上那一格只记得下队头的）。
    at: u64,
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
    /// 这一只（**队头**）的中段告警已经打过了没有（一枚孔最多一行）。
    alarmed: AtomicBool,
    /// **"报不出就绪、又没有手"**那一格的中段告警打过没有（与 `alarmed` 分开：两件事，
    /// 一枚孔各最多一行）。
    stuck: AtomicBool,
}

impl HoleMeta {
    pub(super) fn new(id: HoleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            id,
            life: Life::new(),
            owner,
            pending: SpinLock::new_level(Level::L3, Pending::Idle),
            alarmed: AtomicBool::new(false),
            stuck: AtomicBool::new(false),
        })
    }

    /// **那一格此刻是什么形状**（诊断那一行用；`Queue` 空着也要与"根本不在队列那一档"分开）。
    fn pending_name(&self) -> &'static str {
        match &*self.pending.lock() {
            Pending::Idle => "idle",
            Pending::Queue(q) => {
                if q.hands.is_empty() {
                    // **这一格是那一族的头号嫌疑**：`ready(Push)` 与 `ready(Pull)` **都是假**
                    // ——写的人推不进来（也不会有"手压着"的告警），取的人等不到。
                    "queue-empty"
                } else if q.taking {
                    "queue-taking"
                } else {
                    "queue-hands"
                }
            }
            Pending::Rung => "rung",
        }
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

    /// 就绪：`Pull` = **队头**可取（一只手，或一个已响的位）；`Push` = **队列空着**。
    ///
    /// 两个方向仍按"一只手"的老口径读——**"有位"不是"空"**：写者要"轮到我"时走
    /// `push(…, Wait::POLL)` 那一档（满了答 `Busy`，见 [`give`]）；要"我的手被取走了"时等的
    /// 正是这一格（队列空 ⟺ 单推写者的手全下线了）。把这一格改成"有位"会让四处
    /// `door.wait(HoleDir::Push, Wait::Forever)` 当场变成空等（它们护的是"这段字节活到被取走"）。
    pub(crate) fn ready(&self, dir: HoleDir) -> bool {
        let pending = self.pending.lock();
        match dir {
            HoleDir::Pull => match &*pending {
                Pending::Queue(q) => !q.hands.is_empty() && !q.taking,
                Pending::Rung => true,
                Pending::Idle => false,
            },
            HoleDir::Push => matches!(*pending, Pending::Idle),
        }
    }

}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
        // 孔都没了、手还在 ⇒ 也算"没人取"。**这里只记账、不打印**（`Drop` 可能在别的锁底下跑，
        // 打印要取控制台锁）；结账那一路见 [`note_hand_off`]。
        // **一列手一只不留**：`seal` 那一趟若已经结过账，这里看到的已是空的（它把队列清掉了）。
        if let Pending::Queue(q) = &*self.pending.lock() {
            HANDS_LIVE.fetch_sub(q.hands.len(), Ordering::Relaxed);
        }
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

/// **递出一只手**：排进这一列（先进先出），并唤醒等取的人。
///
/// **不搬字节、不分配**（`VecDeque` 懒分配：只在真排过队之后才向堆要一格）：登记的就是发送方
/// 那一段（`va`/`len`），复制由取的一方做（一处）。
///
/// **满了（[`QUEUE_CAP`]）／位已响 ⇒ `Busy`**：满不丢最旧——"丢不丢"留给写者（`Wait::POLL`
/// 那一档正是为它准备的）。写者要"轮到我有位"就用 `POLL` 重试；`AtMost`／`Forever` 那一档
/// 等的是**队列空**（老语义，见 [`HoleMeta::ready`] 那一节的注）。
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
    let at = clock::uptime_ticks();
    let mut pending = meta.pending.lock();
    if !matches!(*pending, Pending::Idle | Pending::Queue(_)) {
        // 位已响：位与手不共存（与今天同一句）。
        return Err(MailFail::Busy);
    }
    if matches!(*pending, Pending::Idle) {
        *pending = Pending::Queue(Queue {
            hands: VecDeque::new(),
            taking: false,
        });
    }
    let Pending::Queue(q) = &mut *pending else {
        return Err(MailFail::Busy);
    };
    if q.hands.len() >= QUEUE_CAP {
        return Err(MailFail::Busy);
    }
    if q.hands.try_reserve(1).is_err() {
        return Err(MailFail::OoM);
    }
    q.hands.push_back(Hand {
        from,
        space: Arc::downgrade(space),
        va,
        len,
        at,
    });
    HANDS_LIVE.fetch_add(1, Ordering::Relaxed);
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
    let dead = {
        let Pending::Queue(q) = &mut *pending else {
            return Err(MailFail::Busy);
        };
        if q.taking || q.hands.is_empty() {
            return Err(MailFail::Busy);
        }
        if q.hands.front().and_then(|h| h.own()).is_some() {
            q.taking = true;
            return Ok(());
        }
        // **发送方那个空间已经回收**（弱引用升不上来）⇒ 这一只**作废**（消息随人走），
        // 就地让它下线——不作废的话，一位退场的发送方会把这一列的头永久堵死。
        q.hands.pop_front()
    };
    let empty = match &*pending {
        Pending::Queue(q) => q.hands.is_empty(),
        _ => false,
    };
    if empty {
        *pending = Pending::Idle;
    }
    drop(pending);
    if let Some(hand) = dead {
        note_gone(meta, hand.from, hand.va, hand.len, 0, "space");
        note_hand_off(meta, hand.from, hand.len, hand.at);
    }
    if empty {
        let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
    }
    Err(MailFail::Gone)
}

/// 取用中那只手的复制源：`(发送者, 它那段空间, 起点, 长度)`。
///
/// 只在 [`take`] 与 [`taken`]／[`back`] 之间非 `None`——那一段窗口里只有本取用者进得来。
pub(crate) fn source(meta: &HoleMeta) -> Option<(TaskId, Arc<Space>, usize, usize)> {
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Queue(q) if q.taking => q
            .hands
            .front()
            .and_then(|hand| hand.own().map(|(space, va, len)| (hand.from, space, va, len))),
        _ => None,
    }
}

/// **那只手到此为止**：孔回到空闲，唤醒等递的那一方。
///
/// 两个调用点，同一个状态迁移：
/// - **复制成了**（`hand_over` 走到尾）：送到；
/// - **发送方那段已经没了**（`hand_over` 答 `Gone`）：那条报**再也送不到**了——发送方那个空间
///   已经回收，`Hand` 里的只是它剩下的弱引用。**这时必须就地收掉**：照 `back` 那样"手原样
///   放回"会把孔**永远占住**（此后每一次 `Pull` 都答 `Gone` 再把手放回，谁也推不进来）——
///   **实测过**：驱逐之后另一个人 `Push` 永远 `Busy`（无期等就挂住整台机器）。
///
/// **与 [`back`] 的分界**：`back` 是"这一趟没成，但**东西还在**"（装不下 / 缓冲不可写）；
/// 这一格是"**东西没了**"。前者还能拿更大的缓冲再来，后者没有下一趟。
pub(crate) fn taken(meta: &HoleMeta) {
    let mut off = None;
    let mut empty = false;
    {
        let mut pending = meta.pending.lock();
        if let Pending::Queue(q) = &mut *pending {
            if q.taking {
                q.taking = false;
                off = q.hands.pop_front();
                empty = q.hands.is_empty();
            }
        }
        if empty {
            *pending = Pending::Idle;
        }
    }
    if let Some(hand) = off {
        note_hand_off(meta, hand.from, hand.len, hand.at);
    }
    let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
}

/// 复制没成、**东西还在**：这一只手放回**原处**（它本来就是队头，`taking` 清掉即可）。
///
/// 这一条与 `MailCall::Pull` 三条不消费路径是同一条口径：`max` 装不下、缓冲不可写
/// ——**都不替调用方丢东西**（丢一条消息不可逆）。放回之后这一列照旧报就绪（那只手还压着）。
pub(crate) fn back(meta: &HoleMeta) {
    let mut pending = meta.pending.lock();
    if let Pending::Queue(q) = &mut *pending {
        q.taking = false;
    }
}

//     pub(crate) fn withdraw(meta: &HoleMeta, from: TaskId) -> Result<(), MailFail>
//
// ——"把**我自己**伸出、还没被取走的那只手收回来"（`Hand(from) => Idle` ＋ 唤醒 `Push` 侧；
// `Taking` 答 `Busy`，因为那一刻复制在另一颗 hart 上做）。**这只手今天只有"被取走"与
// "随孔一起没"两个下场**，不需要第三条路——三条凭据：①递出的字节今天住在写端那一格
// （`Sender`）里；②孔封印时就地抹手并唤醒发送方（`seal` 那一格）；③`Hand.space` 是弱引用，
// 发送方走了后来那次 `Pull` 答 `Gone`。日后要做"押下—取回"（`Held`）那一类，按它的语义
// 重新定形再加回来。

/// 只看**队头**那一只手：`(长度, 发送者, 队里排着几只)`。**不动状态**（取用中的那只也照报）。
///
/// 第三格是**队列深度**——写者据此知道"我还排着几手"（`hand::Sender` 那一侧就靠它把
/// 自己那几格缓冲收回来），A2 那一格此前空着，故这是**加一格**，A0／A1 的含义一字不动。
pub(crate) fn peek(meta: &HoleMeta) -> Result<(usize, TaskId, usize), MailFail> {
    if !meta.alive() {
        return Err(MailFail::Dead);
    }
    let pending = meta.pending.lock();
    match &*pending {
        Pending::Queue(q) => match q.hands.front() {
            Some(hand) => Ok((hand.len, hand.from, q.hands.len())),
            None => Err(MailFail::Busy),
        },
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
    // **"报不出就绪、又根本没有手"**这一格：等的人（**`POLL` 那一档也在内**——它在下面
    // `dur == ZERO` 那一条就返回了）每一拍都从这儿过，却**一条读数也不落**：`hand_age` 答
    // `None` ⇒ `note_hold` / `alarm_stuck` 两条都不跑；而 `ready(Pull)`／`ready(Push)` 都是假
    // ⇒ 谁也醒不过来。**这一形是量出来的**：accept 景偶发"整机跑完不出场"（canonical 卡在
    // "写口就绪吗"那一问上，60 s 里零读数、连 `idle 10000ms` 那条看门狗都不响——因为等的人
    // 每一毫秒都在跑）。报**孔号 ＋ 主人 ＋ 那一格的形状**，这一族下次就自己报名。
    if dir == HoleDir::Push && hand_age(meta).is_none() {
        // **只认那一格永久的形状**：`Pending::Rung` 是**法定**状态（路由者给"线"那一枚孔置位、
        // 由取的人 `hush`），故写的人在那儿等是常事——量过：正常跑里每跑 1~2 次，报它只会把
        // 信号淹掉。`Queue` **空着**那一格不同：它没有任何一条路能自己回到 `Idle`
        // （`take` 要有手、`taken` 要先 `take`）⇒ 一旦出现就是永久的两头堵死。
        if let Pending::Queue(q) = &*meta.pending.lock()
            && q.hands.is_empty()
        {
            note_stuck(meta);
        }
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    let out = messenger::wait(key(meta, dir), meta.life(), dur)?;
    // **量的是这只手在孔上压了多久，不是这一次 `wait` 睡了多久**——见 [`hold_line`] 那一节。
    // `Push` 方向 = 递出手的那一方在等它下线；`Pull` 方向等的是"有信来"，不记（等信是常态）。
    if dir == HoleDir::Push
        && let Some((from, len, ms)) = hand_age(meta)
    {
        note_hold(meta, from, ms);
        alarm_stuck(meta, from, len, ms);
    }
    Ok(match out {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready(dir)),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

// ── 诊断：一只手在孔上压了多久（"递出去没人取"）────────────────────────────
//
// 今天量的是**一只手自己的寿命**：登记（[`give`] 盖 [`Hand::at`]）→ 下线（[`note_hand_off`] 结账）。
// 它与"某位递手的线程被卡了多久"同值——那一位正是一圈一圈等这只手下线的人。
//
// **下线只有三个点，`back` 不算**：`taken`（复制成了）、`take` 的 `Gone` 支（发送方那段没了、
// 手被就地收掉）、`seal` 与 `Drop`（手随孔作废）。`back` 是"这一趟没成、东西还在"⇒ 手放回，
// 时长接着长（发送方确实还等着）。四条路加起来**没有缺口**：孔上没手时 `hand_age` 答 `None`，
// 而那一刻的终值已经由下线点记下。
//
// **只记不动**：不撤手、不改任何判决、不加期限。
static HOLD_N: AtomicUsize = AtomicUsize::new(0);
static HOLD_MAX_MS: AtomicUsize = AtomicUsize::new(0);
static HOLD_WORST: AtomicUsize = AtomicUsize::new(0);
static HOLD_FROM: AtomicUsize = AtomicUsize::new(0);
static HOLD_OWNER: AtomicUsize = AtomicUsize::new(0);
static HANDS_LIVE: AtomicUsize = AtomicUsize::new(0);
static ALARM_N: AtomicUsize = AtomicUsize::new(0);
/// **"报不出就绪、又没有手"**那一格观测到几次（中段一行 ＋ 收场那一行都读它）。
static STUCK_N: AtomicUsize = AtomicUsize::new(0);
/// **一秒钟**：这只手压了这么久还没人取，就**记账 ＋ 当场报一行**（同一条线，见 [`hold_line`]）。
const HOLD_MS: usize = 1000;
/// 中段告警最多打几行（防洪水：一枚孔一行 ＋ 总量封顶）。
const ALARM_MAX: usize = 8;

/// **一格的形状报不出来、又没有手**：报一行（一枚孔最多一行 ＋ 总量封顶）。
///
/// 它报的正是"**两边都动不了**"那一格：写的人见 `ready(Push)` 为假（推不进来）、取的人见
/// `ready(Pull)` 为假（等不到），而孔上**没有手** ⇒ 连"这只手压了多久"那条读数也没有。
/// 形状由 [`HoleMeta::pending_name`] 说——`queue-empty` 与 `rung` 是两种不同的下一步。
fn note_stuck(meta: &HoleMeta) {
    if meta.stuck.swap(true, Ordering::Relaxed) {
        return;
    }
    STUCK_N.fetch_add(1, Ordering::Relaxed);
    if STUCK_N.load(Ordering::Relaxed) > ALARM_MAX {
        return;
    }
    crate::putln!(
        "mail: push blocked hole#{} owner={} pending={} live={}",
        meta.id.0,
        meta.owner.get(),
        meta.pending_name(),
        HANDS_LIVE.load(Ordering::Relaxed),
    );
}

fn elapsed_ms(at: u64) -> usize {
    clock::ticks_to_duration(clock::uptime_ticks().wrapping_sub(at)).as_millis() as usize
}

/// 孔上那只手压了多久（毫秒）＋ 谁递的 ＋ **有多长**；孔上没手 ⇒ `None`。
///
/// **长度是这一格最认得出"在哪一步"的那一格数**（本仓各族的帧长各不相同：`Said` 那种一格状态
/// 是 2、一枚号是 9、一族 `Reply` 是 10、一条 `land` 两位数是 41/60、一条路是几十……）——故它
/// 跟着 `age` 一起报出来。
fn hand_age(meta: &HoleMeta) -> Option<(TaskId, usize, usize)> {
    let pending = meta.pending.lock();
    let (from, len, at) = match &*pending {
        Pending::Queue(q) => match q.hands.front() {
            Some(hand) => (hand.from, hand.len, hand.at),
            None => return None,
        },
        _ => return None,
    };
    if at == 0 {
        return None;
    }
    Some((from, len, elapsed_ms(at)))
}

/// 记一笔"这一只手压了 `ms` 毫秒还没人取"。**只记账：不取锁、不打印**（`Drop` 也叫它）。
///
/// `HOLD_N` 数的是**观测次数**（每次 Push 方向复探看见它还在就加一），不是手数；
/// 要读的是 `HOLD_MAX_MS` 与 `worst`／`from` —— "最久的那一手压了多久、在哪一枚孔上、谁递的"。
fn note_hold(meta: &HoleMeta, from: TaskId, ms: usize) {
    if ms < HOLD_MS {
        return;
    }
    HOLD_N.fetch_add(1, Ordering::Relaxed);
    if ms > HOLD_MAX_MS.load(Ordering::Relaxed) {
        HOLD_MAX_MS.store(ms, Ordering::Relaxed);
        HOLD_WORST.store(meta.id.0, Ordering::Relaxed);
        HOLD_FROM.store(from.get(), Ordering::Relaxed);
        HOLD_OWNER.store(meta.owner.get(), Ordering::Relaxed);
    }
}

/// 中段告警：一枚孔只打一行、全局封顶 [`ALARM_MAX`] 行。**在孔锁之外叫**（里面有打印）。
fn alarm_stuck(meta: &HoleMeta, from: TaskId, len: usize, ms: usize) {
    if ms < HOLD_MS || meta.alarmed.swap(true, Ordering::Relaxed) {
        return;
    }
    if ALARM_N.fetch_add(1, Ordering::Relaxed) >= ALARM_MAX {
        return;
    }
    crate::putln!(
        "mail: hand stuck hole#{} from={} owner={} len={} age={}ms",
        meta.id.0,
        from.get(),
        meta.owner.get(),
        len,
        ms,
    );
}

/// 这只手**下线了**：把"压了多久"结一次账（**用这一只自己的落手时刻**，见 [`Hand::at`]）。
///
/// **不取锁**：调用方必须已经放下孔锁（里面有打印）。
fn note_hand_off(meta: &HoleMeta, from: TaskId, len: usize, at: u64) {
    if at == 0 {
        return;
    }
    let ms = elapsed_ms(at);
    HANDS_LIVE.fetch_sub(1, Ordering::Relaxed);
    note_hold(meta, from, ms);
    alarm_stuck(meta, from, len, ms);
}

/// 收场那一行（`conductor` 叫；与 `timer:`/`doom:` 同处）。
///
/// `live` = 收场时**还压在孔上、没人取**的手数（>0 = 有手永远没人取）。
/// `back_n` / `back_max_len` = **读不成、手原样放回**的次数与最长那一趟（见 [`note_back`]）。
/// `gone_n` = **这只手送不到**（`Gone`，三格成因见 [`note_gone`]）的次数——**正常应当为 0**：
/// 它数的是"有一条报被内核当场扔掉了"（收方读到的是 `MailFail::Gone`）。
pub(crate) fn hold_line() {
    crate::putln!(
        "hole: push_hold_n={} push_hold_max_ms={} worst=hole#{} from={} owner={} live={} back_n={} back_max_len={} gone_n={} stuck_n={}",
        HOLD_N.load(Ordering::Relaxed),
        HOLD_MAX_MS.load(Ordering::Relaxed),
        HOLD_WORST.load(Ordering::Relaxed),
        HOLD_FROM.load(Ordering::Relaxed),
        HOLD_OWNER.load(Ordering::Relaxed),
        HANDS_LIVE.load(Ordering::Relaxed),
        BACK_N.load(Ordering::Relaxed),
        BACK_MAX_LEN.load(Ordering::Relaxed),
        GONE_N.load(Ordering::Relaxed),
        STUCK_N.load(Ordering::Relaxed),
    );
}

// ── 诊断：**读不成、手原样放回**（`Denied`）────────────────────────────────
//
// 放回之后孔**仍然报就绪**（那只手还压着）⇒ 等在这一组上的读的人还会回来啃这一格、永远读不成；
// 递手那一方还在等"它下线"（`wait(HoleDir::Push, …)` **没有期限**）⇒ 两边一起停住。
// 而**别的客人不再被这一格永远挡住**：内核挑"哪一格就绪"今天从**轮转游标**起扫、取第一枚
// （`envcall/tole.rs::ready` ）——"从头扫、取第一枚 ⇒ 后面几位客人的手被饿在后面"
// 那一形是 `76ce954` 修的。故这一格必须看得见：第一次当场报一行（被 host 杀掉的跑走不到收场块），
// 次数与最长长度留收场那一行。
static BACK_N: AtomicUsize = AtomicUsize::new(0);
static BACK_MAX_LEN: AtomicUsize = AtomicUsize::new(0);
static BACK_ALARMED: AtomicBool = AtomicBool::new(false);

/// 记一笔"这一只手**读不成、原样放回**"（`Denied`：`len > max` 装不下，或收方缓冲不可写）。
pub(crate) fn note_back(len: usize, max: usize) {
    BACK_N.fetch_add(1, Ordering::Relaxed);
    BACK_MAX_LEN.fetch_max(len, Ordering::Relaxed);
    if !BACK_ALARMED.swap(true, Ordering::Relaxed) {
        crate::putln!("mail: hand returned len={} max={}", len, max);
    }
}

// ── 诊断：**这只手送不到了**（`Gone`）──────────────────────────────────────
//
// **（这一行要证的那件事：`Gone` 有三个成因，而它们下一步完全不同）**：
//
// 三格都折成同一个 `MailFail::Gone`，而"人走了"与"内存没了"要修的地方**不在同一层**。
// 故第一次当场报一行（被 host 杀掉的跑走不到收场块），把 `from` / `va` / `len` / `dst` 四格一起
// 报出来——验收与调试两边都能拿它对上"是哪一位、哪两段"。
static GONE_N: AtomicUsize = AtomicUsize::new(0);
static GONE_ALARMED: AtomicBool = AtomicBool::new(false);

/// 报一笔"这只手送不到了"（成因见上，`why` 就是那一句话）。
///
/// `dst` = 收方那一段的 VA（0 = 这一格与收方缓冲区无关）——**两边在一页里的偏移不同**正是
/// `why=copy` 那一格要看的东西。
pub(crate) fn note_gone(
    meta: &HoleMeta,
    from: TaskId,
    va: usize,
    len: usize,
    dst: usize,
    why: &str,
) {
    GONE_N.fetch_add(1, Ordering::Relaxed);
    if !GONE_ALARMED.swap(true, Ordering::Relaxed) {
        crate::putln!(
            "mail: hand gone hole#{} from={} owner={} va={:#x} len={} dst={:#x} why={}",
            meta.id.0,
            from.get(),
            meta.owner.get(),
            va,
            len,
            dst,
            why,
        );
    }
}

pub(crate) fn seal(meta: &HoleMeta) {
    *meta.state.lock() = HoleState::Dead;
    // 手随孔作废 —— 也是一种"没人取"（结账在放下孔锁之后：`note_hand_off` 里有打印）。
    // **一列手一只不留**：队头按它自己那一刻结账、余下几只各减一枚，队列就地清掉
    // （不清的话 `Drop` 会再结一次账 ⇒ `HANDS_LIVE` 下溢——实测踩过）。
    let (head, rest) = {
        let mut pending = meta.pending.lock();
        let (head, rest) = match &*pending {
            Pending::Queue(q) => (
                q.hands.front().map(|h| (h.from, h.len, h.at)),
                q.hands.len().saturating_sub(1),
            ),
            _ => (None, 0),
        };
        *pending = Pending::Idle;
        (head, rest)
    };
    if let Some((from, len, at)) = head {
        note_hand_off(meta, from, len, at);
    }
    HANDS_LIVE.fetch_sub(rest, Ordering::Relaxed);
    messenger::wipe(key(meta, HoleDir::Pull));
    messenger::wipe(key(meta, HoleDir::Push));
}

pub(crate) fn meta(owner: TaskId) -> Arc<HoleMeta> {
    HoleMeta::new(alloc_id(), owner)
}

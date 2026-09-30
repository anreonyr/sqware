use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    /// 手上那一刻（ticks；`0` = 孔上没手）。**诊断用**：见 [`note_hand_off`] 那一节。
    hand_at: AtomicU64,
    /// 这只手的中段告警已经打过了没有（一枚孔最多一行）。
    alarmed: AtomicBool,
}

impl HoleMeta {
    pub(super) fn new(id: HoleId, owner: TaskId) -> Arc<Self> {
        Arc::new(Self {
            state: SpinLock::new_level(Level::L3, HoleState::Live),
            id,
            life: Life::new(),
            owner,
            pending: SpinLock::new_level(Level::L3, Pending::Idle),
            hand_at: AtomicU64::new(0),
            alarmed: AtomicBool::new(false),
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

    /// 孔上那只手：**谁递的 ＋ 有多长**；孔上没手（或只是个响着的位）⇒ `None`。**只读**。
    ///
    /// `Taking`（正被取用）**也算在手上**——那一瞬复制在另一颗 hart 上做，手还没下线。
    /// 长度只服务诊断（[`alarm_stuck`] 那一行认"在哪一步"靠它）。
    fn held(&self) -> Option<(TaskId, usize)> {
        let pending = self.pending.lock();
        match &*pending {
            Pending::Hand(hand) | Pending::Taking(hand) => Some((hand.from, hand.len)),
            _ => None,
        }
    }
}

impl Drop for HoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = HoleState::Dead;
        // 孔都没了、手还在 ⇒ 也算"没人取"。**这里只记账、不打印**（`Drop` 可能在别的锁底下跑，
        // 打印要取控制台锁）；结账那一路见 [`note_hand_off`]。
        if let Some((from, _len)) = self.held() {
            let at = self.hand_at.swap(0, Ordering::Relaxed);
            if at != 0 {
                HANDS_LIVE.fetch_sub(1, Ordering::Relaxed);
                note_hold(self, from, elapsed_ms(at));
            }
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

/// 递出一只手：孔空着就登记并唤醒等取的人。
///
/// **不搬字节、不分配**：登记的就是发送方那一段（`va`/`len`），复制由取的一方做（一处）。
/// 孔上已有手／正被取用／位已响 ⇒ `Busy`。
///
/// **判死只有一格：`meta.alive()`**。这里原先还问一句"主人还在吗"（`muster` 直查清册）
/// ——**照实记（已经退场了，附判决）**：那一问补的是"封印那一趟没走到"的漏，而漏的根因
/// 不在名册轴，在**封印当时要先把人找回来**（`gate::doom` 走 `snap::snap()` ＋ `snap::find`，
/// 而 `roster()` 自己要分配、备不出容量就返回空表 ⇒ 那趟一枚都不封，"主人走了、资源还活着"
/// 恰好落在内存最紧的一刻）。把退场钩子的签名从 `fn(TaskId)` 改成 `fn(&Arc<Task>)`
/// （`reap` 手上本来就握着那一具）之后，封印不再看快照，缝就没了：
/// **摘掉这一格，七景两趟全绿**（判决那一趟 root／product 2.22 s、rig 3.13 s、group 0.37 s；
/// 收尾那一趟 product 量到 4.23 s——收场由喂入帮手每 2 s 重喂驱动，读数是量化过的，
/// **判据是红绿**）。留着它等于在热路上多取一次清册锁，却只答一个状态轴已经答过的问题。
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
    // 这一只手**从这一刻起算**（诊断：见 [`note_hand_off`] 那一节）。每次 `give` 重写，
    // 上一只手的时长不会被带过来。
    meta.hand_at.store(clock::uptime_ticks(), Ordering::Relaxed);
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
    let cur = core::mem::replace(&mut *pending, Pending::Idle);
    match cur {
        Pending::Hand(hand) => match hand.own() {
            Some(_) => {
                *pending = Pending::Taking(hand);
                Ok(())
            }
            None => {
                let from = hand.from;
                let (va, len) = (hand.va, hand.len);
                drop(pending);
                note_gone(meta, from, va, len, 0, "space");
                note_hand_off(meta, from, len);
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
    let mut pending = meta.pending.lock();
    let mut held = None;
    if let Pending::Taking(hand) = &*pending {
        held = Some((hand.from, hand.len));
        *pending = Pending::Idle;
    }
    drop(pending);
    if let Some((from, len)) = held {
        note_hand_off(meta, from, len);
    }
    let _ = messenger::wake(key(meta, HoleDir::Push), &meta.life());
}

/// 复制没成、**东西还在**：手原样放回。
///
/// 这一条与 `MailCall::Pull` 三条不消费路径是同一条口径：`max` 装不下、缓冲不可写
/// ——**都不替调用方丢东西**（丢一条消息不可逆）。
///
/// **照实记（这里少了一格，量出来的）**：从前这一条也管第三件事"对面那段没了"，而那条路是
/// **错的**：发送方已走，那条报不可能再送出去，放回只会把孔占死（见 [`taken`] 的照实记）。
/// 今天"对面那段没了"走 [`taken`]（收掉），这一格只管"还能再来一趟"的两条。
pub(crate) fn back(meta: &HoleMeta) {
    let mut pending = meta.pending.lock();
    let cur = core::mem::replace(&mut *pending, Pending::Idle);
    match cur {
        Pending::Taking(hand) => *pending = Pending::Hand(hand),
        other => *pending = other,
    }
}

// **照实记（`withdraw` 那一格退了场）**：这里原先是
//
//     pub(crate) fn withdraw(meta: &HoleMeta, from: TaskId) -> Result<(), MailFail>
//
// ——"把**我自己**伸出、还没被取走的那只手收回来"（`Hand(from) => Idle` ＋ 唤醒 `Push` 侧；
// `Taking` 答 `Busy`，因为那一刻复制在另一颗 hart 上做）。它的唯一用家是载体那层的兜底期限
// （`HolePie::push` 的 `HANDOFF_MS`），那一条也随本刀退了场。三条凭据（原话见
// `runtime::env::mail` 光标处与 `env::fid` 的 `MailCall` 那一节）：①递出的字节今天住在
// 写端那一格（`Sender`）里；②孔封印时就地抹手并唤醒发送方（`seal` 那一格）；③`Hand.space` 是弱引用，
// 发送方走了后来那次 `Pull` 答 `Gone`。**这只手因此只有"被取走"与"随孔一起没"两个下场**，
// 不需要第三条路。日后要做"押下—取回"（`Held`）那一类，按它的语义重新定形再加回来。

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
    let out = messenger::wait(key(meta, dir), meta.life(), dur)?;
    // **量的是这只手在孔上压了多久，不是这一次 `wait` 睡了多久**——照实记见 [`hold_line`] 那一节。
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
// **照实记（这一版量错了，量不出来的那一格在哪儿）**：上一版量的是**单次 `wait`** 的时长
// ——`note_hold(dir, 本次 wait 睡了多久)`。而单次 `wait` 被内核收在 [`BLIND_MS`]（~100 ms，
// 见 `runtime::chrono::timer`），载体那层的 `wait(HoleDir::Push, …)` 又是一个"睡一小段、醒来再探"的循环
// ⇒ **每一次测得的都 ≤ ~100 ms，永远够不到 [`HOLD_MS`]**。于是一位发件人**被一只手卡满 60 秒，
// 收场那行照样打 `push_hold_n=0 push_hold_max_ms=0`**——那不是"没人等"，是**这把尺子量不到**。
//
// 今天量的是**一只手自己的寿命**：登记（[`give`] 盖 `hand_at`）→ 下线（[`note_hand_off`] 结账）。
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
/// **一秒钟**：这只手压了这么久还没人取，就**记账 ＋ 当场报一行**（同一条线，见 [`hold_line`]）。
const HOLD_MS: usize = 1000;
/// 中段告警最多打几行（防洪水：一枚孔一行 ＋ 总量封顶）。
const ALARM_MAX: usize = 8;

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
    let (from, len) = match &*pending {
        Pending::Hand(hand) | Pending::Taking(hand) => (hand.from, hand.len),
        _ => return None,
    };
    let at = meta.hand_at.load(Ordering::Relaxed);
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
///
/// # 照实记（**门槛从 2000 ms 收到 [`HOLD_MS`]：这一格从前是哑的**）
///
/// 它原先是**另一个数**（2000 ms），而 [`note_hold`] 记的是 ≥1000 ms——**两把尺子量同一件事**，
/// 于是出现这一形：debug 档 `product` 景收场那一行写着
/// `push_hold_n=1..6 push_hold_max_ms≈1117~1732 worst=hole#24x from=20 owner=20`
/// ——**账上记着，却一行也没报**（最久 1732 ms < 2000 ms 的门槛），"是哪一枚孔、压在哪一步"
/// 一个字都读不到。今天并成一条线：**记了账的就报得出一行**（同一枚孔仍最多一行、全局封顶）。
/// **它是在"还压着"的时候打的**（调用点在 [`wait`] 的复探上，不是下线时）——故那一行在串口上
/// 的**位置**就是"当时机器在忙什么"。
/// `len` 一并报出来：各族帧长不同，它是这一格最认得出"在哪一步"的那一格数（见 [`hand_age`]）。
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

/// 这只手**下线了**：把"压了多久"结一次账，清掉时间戳（`swap` 保证同一只手只结一次）。
///
/// **不取锁**：调用方必须已经放下孔锁（里面有打印）。
fn note_hand_off(meta: &HoleMeta, from: TaskId, len: usize) {
    let at = meta.hand_at.swap(0, Ordering::Relaxed);
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
        "hole: push_hold_n={} push_hold_max_ms={} worst=hole#{} from={} owner={} live={} back_n={} back_max_len={} gone_n={}",
        HOLD_N.load(Ordering::Relaxed),
        HOLD_MAX_MS.load(Ordering::Relaxed),
        HOLD_WORST.load(Ordering::Relaxed),
        HOLD_FROM.load(Ordering::Relaxed),
        HOLD_OWNER.load(Ordering::Relaxed),
        HANDS_LIVE.load(Ordering::Relaxed),
        BACK_N.load(Ordering::Relaxed),
        BACK_MAX_LEN.load(Ordering::Relaxed),
        GONE_N.load(Ordering::Relaxed),
    );
}

// ── 诊断：**读不成、手原样放回**（`Denied`）────────────────────────────────
//
// 放回之后孔**仍然报就绪**（那只手还压着）⇒ 等在这一组上的读的人还会回来啃这一格、永远读不成；
// 递手那一方还在等"它下线"（`wait(HoleDir::Push, …)` **没有期限**）⇒ 两边一起停住。
// 而**别的客人不再被这一格永远挡住**：内核挑"哪一格就绪"今天从**轮转游标**起扫、取第一枚
// （`envcall/tole.rs::ready` 的照实记）——"从头扫、取第一枚 ⇒ 后面几位客人的手被饿在后面"
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
// **照实记（这一行要证的那件事：`Gone` 有三个成因，而它们下一步完全不同）**：
//
//   · **`why=space`**——登记时那个**空间**已经回收（`Weak` 升不上来，"消息随人走"）：说明递手的
//     那一位**已经退场**，或它的那一叠（team）已经没了；
//   · **`why=range`**——空间还在，可**那段 VA 今天不可读**（`whole(.., R)` 不过，复制没敢做）：
//     说明那只手**指着一段已经不在了的内存**（栈被人复用、或那段页被收走）——那正是本仓栽过
//     两次的那一类（"递出去的那一段"没活到被取走）；
//   · **`why=copy`**——过得了检查、复制却失败了：`copy` 那一段自己再验一遍两面（源可读、收可写）
//   并逐片段搬；它答 `false` 的那一格**正是两面片段边界对不齐**（照实记见 `work/mail/mod.rs`
//   的 `copy`）。
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
pub(crate) fn note_gone(meta: &HoleMeta, from: TaskId, va: usize, len: usize, dst: usize, why: &str) {
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
    if let Some((from, len)) = meta.held() {
        note_hand_off(meta, from, len);
    }
    messenger::wipe(key(meta, HoleDir::Pull));
    messenger::wipe(key(meta, HoleDir::Push));
}

pub(crate) fn meta(owner: TaskId) -> Arc<HoleMeta> {
    HoleMeta::new(alloc_id(), owner)
}

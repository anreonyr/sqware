//! `session::call` 的**桩** —— 会话核心要的那十件手（[`super::hands::Hands`]）都在这儿：
//!
//! ```text
//!   mint(mark)          铸一枚孔（刻上记号）
//!   ship(hole, peer)    把它交给对端（Accord 一份副本）
//!   unship(hole)        放下我这一份
//!   post / try_post     往一枚孔推一句话
//!   pull_own            从我自己那一枚收一句话
//!   reserve(hole)       这枚孔的两格事实：谁开的 / 刻的什么
//!   each(f)             扫我这张表
//!   fall(ms) / now_ns   等与时钟
//! ```
//!
//! **桩量不了内核，量得了账**：下面是一张**进程内**的假表（线程局部 ⇒ libtest 每个用例各一枚
//! 线程，用例之间互不相干）加一枚**假钟**（`fall` 把钟往前拨，故"有界等"在宿主上是确定性的）。
//!
//! 假表里一行 = 内核那张表里的一枚孔：`token` / `owner`（谁开的）/ `mark`（刻的什么）。
//! `mint` 往里**加**一行（主人是"本端"），测试可以用 [`put`] 往里放任意两格的孔——这正是
//! `scan` 那两格判据要辨别的东西。

use std::cell::{Cell, RefCell};

use env::{Mark, Name, PieToken, TaskId};

use super::core::Claim;
use super::hands::{Hands, Hole};

/// "本端"是谁（`mint` 铸出来的那些孔算在它名下）。
pub const ME: TaskId = TaskId::new(1);

/// 假表里的一行。
#[derive(Clone, Copy)]
pub struct Row {
    pub token: PieToken,
    pub owner: Option<TaskId>,
    pub mark: Mark,
}

thread_local! {
    static TABLE: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<usize> = const { Cell::new(1) };
    static CLOCK: Cell<u64> = const { Cell::new(0) };
    static SHIPPED: RefCell<Vec<(PieToken, TaskId)>> = const { RefCell::new(Vec::new()) };
    static UNSHIPPED: RefCell<Vec<PieToken>> = const { RefCell::new(Vec::new()) };
    static SAID: RefCell<Vec<(PieToken, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    static INBOX: RefCell<Vec<(PieToken, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    static REFUSE: Cell<bool> = const { Cell::new(false) };
}

// ── 测试侧的手 ──────────────────────────────────────────────

/// 把这一台清干净（每个用例开头叫一次）。
pub fn reset() {
    TABLE.with(|t| t.borrow_mut().clear());
    NEXT.with(|n| n.set(1));
    CLOCK.with(|c| c.set(0));
    SHIPPED.with(|s| s.borrow_mut().clear());
    UNSHIPPED.with(|s| s.borrow_mut().clear());
    SAID.with(|s| s.borrow_mut().clear());
    INBOX.with(|i| i.borrow_mut().clear());
    REFUSE.with(|flag| flag.set(false));
}

/// 把本线程的 `post` / `try_post` 打成失败（或放开）。
///
/// **照实记（这一格搬过一次家）**：它原本长在 `line` 靶**自己那个 `Pier` 桩**里——`deliver`
/// 那条"推不出去 ⇒ 不置忙"的契约，就是靠这个开关才量得出来（桩恒答 `Ok` 的那一版全门照绿）。
/// `line` 靶改成真 `Pier` 之后，这一格跟着搬到**假手表**上：判据要的那件事（`post` 会失败）
/// 在真的 `Pier::post` 上照样成立，故它没有消失，只是换了一层。
pub fn refuse(on: bool) {
    REFUSE.with(|flag| flag.set(on));
}

/// 造一枚号（**唯一的门是"收号"**，与另外几台同一条）。
pub fn tok(n: usize) -> PieToken {
    PieToken::from_bytes(&(n as u64).to_le_bytes()).expect("8 字节")
}

/// 往假表里放一枚孔：**两格由测试说了算**（`scan` 判的就是这两格）。
pub fn put(n: usize, owner: TaskId, mark: Mark) {
    let token = tok(n);
    TABLE.with(|t| {
        t.borrow_mut().retain(|r| r.token != token);
        t.borrow_mut().push(Row {
            token,
            owner: Some(owner),
            mark,
        });
    });
}

/// 那一枚还在不在我表里（`reserve` 答得出就是"在"）。
pub fn present(n: usize) -> bool {
    TABLE.with(|t| t.borrow().iter().any(|r| r.token == tok(n)))
}

/// 交给对端的那些（按先后）。
pub fn shipped() -> Vec<(PieToken, TaskId)> {
    SHIPPED.with(|s| s.borrow().clone())
}

/// 放下的那些（按先后）。
pub fn unshipped() -> Vec<PieToken> {
    UNSHIPPED.with(|s| s.borrow().clone())
}

/// 推出去的那些话（按先后）。
pub fn said() -> Vec<(PieToken, Vec<u8>)> {
    SAID.with(|s| s.borrow().clone())
}

/// 给本端某一枚孔备一句话（`pull` 收它）。
pub fn put_inbox(n: usize, bytes: &[u8]) {
    INBOX.with(|i| i.borrow_mut().push((tok(n), bytes.to_vec())));
}

/// 把这枚孔从我表里拿走（不记"放下"——那是 `unship` 的事）。
pub fn forget(n: usize) {
    let token = tok(n);
    TABLE.with(|t| t.borrow_mut().retain(|r| r.token != token));
}

// ── 核心要的那几句话（形状与真那份一致）────────────────────

/// 拆泊位那句话**跟着据走了**（它是一句话，不是一只手）——这里转出来给测试用。
///
/// **照实记（`#[allow(unused_imports)]`）**：这一份假手表是**两台共读**的（`quay` 靶与 `line` 靶），
/// 而 `line` 靶不拆泊位 ⇒ 对它来说这一行是空转的。挂 allow 而不是删：删了 `quay` 靶就编不过。
#[allow(unused_imports)]
pub use super::core::UNSEAT;

/// 铸一枚孔、刻上记号：往假表里加一行，主人是"本端"。
pub fn unseal_hole(mark: Mark) -> Result<PieToken, ()> {
    let n = NEXT.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    let token = tok(n);
    TABLE.with(|t| {
        t.borrow_mut().push(Row {
            token,
            owner: Some(ME),
            mark,
        })
    });
    Ok(token)
}

/// 交给对端：记一笔，返"种在对端表里"的号（真那一手是 `Accord` 一份副本）。
///
/// **照实记**：从前的影子桩返 `Result<(), ()>`，而真那份返 `Result<PieToken, ()>`——照样编得过。
/// 表化之后这一个字也漂不了。
pub fn ship(hole: PieToken, peer: TaskId) -> Result<PieToken, ()> {
    SHIPPED.with(|s| s.borrow_mut().push((hole, peer)));
    Ok(hole)
}

/// 放下我这一份：从我表里拿走 + 记一笔。
pub fn unship(hole: PieToken) -> Result<(), ()> {
    UNSHIPPED.with(|s| s.borrow_mut().push(hole));
    TABLE.with(|t| t.borrow_mut().retain(|r| r.token != hole));
    Ok(())
}

/// 推一句话（等的那一版）。`refuse(true)` 时答错——见那个开关的照实记。
pub fn post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    if REFUSE.with(Cell::get) {
        return Err(());
    }
    SAID.with(|s| s.borrow_mut().push((at_peer, msg.to_vec())));
    Ok(())
}

/// 推一句话（满了当场答错的那一版）——桩里两版同效（假表不会满）。
pub fn try_post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    post(at_peer, msg)
}

/// 从我这一枚收一句话（有就取走，没有就报期限内没等到）。
pub fn pull_own(hole: PieToken, buf: &mut [u8], _millis: usize) -> Result<usize, ()> {
    let taken = INBOX.with(|i| {
        let mut inbox = i.borrow_mut();
        inbox
            .iter()
            .position(|(t, _)| *t == hole)
            .map(|at| inbox.remove(at).1)
    });
    match taken {
        Some(bytes) if bytes.len() <= buf.len() => {
            buf[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
        _ => Err(()),
    }
}

/// 这枚孔的两格事实（不在我表里 ⇒ `(None, Mark::NONE)`，与真那份"这一条候选不成立"同调）。
pub fn reserve(hole: PieToken) -> (Option<TaskId>, Mark) {
    TABLE.with(|t| {
        t.borrow()
            .iter()
            .find(|r| r.token == hole)
            .map(|r| (r.owner, r.mark))
            .unwrap_or((None, Mark::NONE))
    })
}

/// 扫我这张表。
///
/// **先照一张相再回调**：回调（`scan` 那一支）会反问 `reserve`，而那也是借这张表——
/// 抱着 `RefCell` 的借用去回调会当场 panic（真那一份是"表在自己手里"，没有这一层）。
pub fn each(f: &mut dyn FnMut(Hole) -> Result<(), Claim>) -> Result<(), Claim> {
    let snapshot: Vec<Row> = TABLE.with(|t| t.borrow().clone());
    for r in snapshot {
        f(Hole {
            token: r.token,
            owner: r.owner,
            mark: r.mark,
        })?;
    }
    Ok(())
}

/// 有界等：**假钟往前拨 `millis`**，故"期限到"在宿主上是确定性的（返回 `false` = 没被叫醒）。
pub fn fall(millis: usize) -> bool {
    CLOCK.with(|c| c.set(c.get().saturating_add(millis as u64 * 1_000_000)));
    false
}

/// 单调钟（纳秒）。
pub fn now_ns() -> u64 {
    CLOCK.with(Cell::get)
}

/// 名字（测试侧顺手用）。
pub fn name(text: &str) -> Name {
    Name::new(text).expect("名字合法")
}

// ── 这一层唯一的出口：一张假手表 ─────────────────────────────

/// 十件手，全在假表上（真那份是 `protocol::session::call::hands`）。
pub fn hands() -> Hands {
    Hands {
        post,
        try_post,
        pull_own,
        unseal: unseal_hole,
        ship,
        unship,
        each,
        reserve,
        fall,
        now_ns,
    }
}

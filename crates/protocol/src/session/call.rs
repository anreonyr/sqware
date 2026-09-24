//! session 的转发层 —— 内核那几只手的别名。
//!
//! 本文件**不做裁决**：全会话的规矩（额度、齐没齐、谁的孔归谁）都在
//! [`core`](super::core)。这里只做两件事：**转发一次**、把内核的错误码翻成"没成"。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` 只翻**内核已经答完的格子**，一格不多：两个 `0` 哨兵
//! > （[`each`] 的"扫完了"、[`reserve`] 的"查不出"）。
//!
//! （旧注写的是"这里不出现 `if` / `match`"——**照实记：这两处哨兵与它同一次落地，
//! 那句话从写下的第一天起就是假的**。）
//!
//! # 这里也是全层共享身体的家
//!
//! 有四个身体原先在板 / 树 / 线各抄一份，且各起一个名；它们现在只住这里，其余模块
//! `use … as …` 取自己那一侧的名字（名字可以因领域而异，**身体不能**）：
//!
//! | 概念 | 地板词 | 共享体（本文件） |
//! |---|---|---|
//! | 交出（授出副本） | `port::ship` | [`ship`] |
//! | 自释一份 | `PieCall::Release` | [`unship`]（`ship` 的反面：装运 / 卸下） |
//! | `Reserve` 三格 | `vestor` / `owner` / `mark` | [`vested_by`] / [`opened_by`] / [`marked_as`] |
//!
//! 三格是一**组**：三个名字读成同一句式的被动式事实（*这枚是谁授的 / 这扇门是谁开的 /
//! 这枚被标成什么*），故**等长**（9/9/9）——原先板与树是 `probe`5 / `opened_by`9 /
//! `mark_of`7，不等长本身就是"这一组还没想清楚"的信号。

use env::{Mark, PieToken, TaskId};

use super::core::Claim;

use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

/// 铸一枚孔（本端那一枚），并把**记号**刻在它上面。
pub(super) fn mint(mark: Mark) -> Result<PieToken, ()> {
    mail::unseal_hole(mark).map_err(|_| ())
}

/// **交出**：一枚副本种进对端表里，返**种在它表里的那个号**。
///
/// 这一族只有三个名字：**`ship` 交出 / `take` 接手 / `unship` 放下**（[`unship`] 是它的反面）。
/// 门闩层那个锚叫 `Pie.heir`——**那在核里，名字照旧**（本层不复述、也不重名它）。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此**可以再授出**。那一格不是客气——
/// 内核的 `Accord` 有一道闸是"持 `VEST` 才交得出去"（`Need::Grant`），而"对端把这一枚
/// 转给第三方"是本结构里**必然**发生的一步：子方只认得它的生我者，故它交出来的孔先落在
/// 生我者表里，再由生我者转授（见正文事实 1 的推论；板那条路就是这么接上的）。
/// 不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对——看上去像"板坏了"。
pub fn ship(hole: PieToken, peer: TaskId) -> Result<PieToken, ()> {
    let pie = mail::HolePie::from_token(hole);
    port::ship(&pie, peer, Access::FETCH | Access::STORE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| ())
}

/// 卸下本端那一枚孔——[`ship`] 的反面：**装运 / 卸下**。
pub fn unship(hole: PieToken) -> Result<(), ()> {
    mail::release(hole).map_err(|_| ())
}

/// 往**对端**那一条泊位说一句话（号是**种在它表里**的那一枚）。
pub(super) fn post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    mail::HolePie::from_token(at_peer).push(msg).map_err(|_| ())
}

/// 同 [`post`]，但槽满**当场**答 `Err`（`HolePie::push` 会等槽空，这一条不）。
pub(super) fn try_post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    mail::push(at_peer, msg.as_ptr(), msg.len()).map_err(|_| ())
}

/// 扫**我表里的每一枚**孔，逐枚交给 `f`。
///
/// **不设上限**：表里可能已经有几十枚（设备门闩、别人给的副本……），而认领只关心其中
/// 的一两枚——设一个"最多看几枚"的缓冲会让排在后面的那枚永远看不见。故这里不攒数组，
/// 只把每一枚交出去；"要不要"由调用方说（正文第 6 条）。
///
/// `Err(Claim::Unread)` = 枚举本身失败（我的表读不动了）。
pub(super) fn each(mut f: impl FnMut(Hole) -> Result<(), Claim>) -> Result<(), Claim> {
    let mut index = 0usize;
    loop {
        let Ok((token, _permission, _grantor)) = mail::collect(index) else {
            return Err(Claim::Unread);
        };
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return Ok(());
        }
        index += 1;
        let (owner, mark) = reserve(token);
        f(Hole { token, owner, mark })?;
    }
}

/// 我表里**这位给的、刻着那个记号的那一枚**。
///
/// 判据就是 [`Quay::claim`](super::core::Quay::claim) 认领时用的那两格正判据
/// （`owner` + `mark`），只是**不要一座码头**：一问一答那一档里，客人每趟借一枚回信孔过来、
/// 交完就推帧，收的人手里没有泊位可归位，只要那一枚号。
///
/// **多枚时给最后那一枚**——表内次序是"什么时候进来的"（`each` 走的是本端那张表），而
/// 交孔与推帧是**两趟**、且**孔先到**（见 [`ship`] 的调用点）⇒ 最后那一枚就是这一趟那一枚。
/// 这条次序是契约的一半，不是实现细节。
///
/// 枚举本身读不动（[`Claim::Unread`]）时报 `None`：那一格里已经有"我没找到"。
pub fn find(of: TaskId, mark: Mark) -> Option<PieToken> {
    let mut found = None;
    let _ = each(|h| {
        if h.owner == Some(of) && h.mark == mark {
            found = Some(h.token);
        }
        Ok(())
    });
    found
}

/// **借一枚回信孔过去、把这一帧推上那扇门**——一问一答的共享体；返**本端那一枚**
/// （答话从那枚孔回来）。
///
/// 三件事，**次序是契约的一半**：先铸、先交（`port::ship`），**再**推帧。收的那一侧按
/// "谁给的 + 记号"两格认（[`find`]），多枚时取**最后**那一枚——故最后那一枚一定就是这一趟
/// 那一枚。
///
/// `entry` = 对端那一扇门（树上查回来的门牌）；`mark` = 回信孔的记号，**各面自己的**
/// （rtc 是 `rtc-back`、principal 是 `principal-back`）：同一张表里两面的回信孔若刻同一个
/// 记号，就分不出这一枚是哪一面的。交出去的权只要 `STORE`——对端只推，读的一侧是本端。
///
/// 对端的号从**这一枚门闩自己**问出来（[`opened_by`]）：门牌是别人挂的，故只能读它；
/// 副本共享同一事实、转手不变。
///
/// **第二例到了**（rtc 那一面的客侧先写过一份，principal 是第二个用家）——照本文件开头的
/// 纪律，身体搬到这里，两处只留各自的名字。
pub fn lend(entry: PieToken, mark: Mark, frame: &[u8]) -> Result<PieToken, ()> {
    let host = opened_by(entry).ok_or(())?;
    let back = mint(mark)?;
    if port::ship(
        &mail::HolePie::from_token(back),
        host,
        Access::STORE,
        Policy::NONE,
    )
    .is_err()
    {
        let _ = mail::release(back);
        return Err(());
    }
    if mail::HolePie::from_token(entry).push(frame).is_err() {
        let _ = mail::release(back);
        return Err(());
    }
    Ok(back)
}

/// 我表里的一项：一枚孔 + **谁开的** + **刻的什么记号**（[`each`] 交出来的那两格）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Hole {
    pub token: PieToken,
    /// 这扇门谁开的（`None` = 查不出出处，如引导期那批设备门闩）。
    pub owner: Option<TaskId>,
    /// 这条路上刻的记号（[`Mark::NONE`] = 这一枚不是孔、问不到记号）。
    pub mark: Mark,
}

/// 这枚孔的两格事实（**一次 `Reserve` 取回**）：**谁开的** + **刻的什么记号**。
///
/// `owner`（`Reserve` 第二格）= 这扇门谁开的：副本共享同一事实，转手不变。
/// `mark`（同一次调用捎回来的那一格）= 铸者刻在孔上的那个名字，同样随副本过线。
/// 认"对端放进来的那一枚"就认这两格：本端 `mint` 出来的 owner 是本端、记号是本端刻的
/// 那个；对端 `ship` 进来的 owner 是对端——**编号比不出来，这两格比得出来**。
///
/// 记号收在**栈上 [`NAME_LEN`](env::NAME_LEN) 字节**的缓冲里（不分配），随同一次调用
/// 拷出（装不下就没有答案）。
///
/// 答不出的那一类——**这一枚不是孔**（Pole/Nole/Tole 上没有记号）、它已不在表里、
/// `owner` 是 0（引导期那批设备门闩）——一律报 `(None, Name::EMPTY)`：`Name::EMPTY` 是
/// "占位、不是合法名"的那一格，故**这一条候选永不匹配**（规格里没有"无名孔"这一态，
/// 这里说的是"这一条候选不成立"）。
pub(super) fn reserve(hole: PieToken) -> (Option<TaskId>, Mark) {
    match mail::reserve(hole) {
        Ok((_vestor, owner, mark)) if owner.get() != 0 => (Some(owner), mark),
        _ => (None, Mark::NONE),
    }
}

// ── `Reserve` 的三格：一个调用的三个事实，一组三个等长名 ──────────────

reserve_reads! {
    /// **这枚是谁授的**（`Reserve` 第一格）。
    ///
    /// 转手（`Accord`）会改写这一格（root 转授过的门闩，`vestor` 会变成 root），故
    /// **不能用它认"对端是谁"**；要认"这扇门本身是谁的"，读 [`opened_by`]。
    ///
    /// **"答不出"这一格里就有"那扇门封印了"**：`Reserve` 的 `owner` 那一格带存活闸
    /// ⇒ 开者一退场，它开的门随之封印 ⇒ 这里当场答 `None`。故 `None` 只读作
    /// "这一条候选不成立"（不在我表里 / 不是孔 / 已封印），**不必再问第二个问题**。
    pub fn vested_by(entry) => vestor;
}

reserve_reads! {
    /// **这扇门是谁开的**（`Reserve` 第二格）。副本共享同一事实，转手不变。
    ///
    /// 回答"这一位客人自己交来的那一枚"就靠它；与 [`marked_as`] 合起来才分得开
    /// "同一位开的多枚孔"（那一格答"这是哪条路上的"）。
    ///
    /// 问不到那两格（这一枚**不是孔**、或它已不在表里）⇒ `None`：这一条候选不成立。
    pub fn opened_by(hole) => owner;
}

reserve_reads! {
    /// **这枚被标成什么记号**（`Reserve` 第三格）。铸者刻在孔上，副本共享、转手不变。
    ///
    /// 持板者 / 持树者那一侧三处认法都读它，各自问同一句——"这是哪条路上的那一枚"
    /// （答话路 / 问话孔 / 注册入口）。
    ///
    /// **不能实现成 [`reserve`] 的第二格**：那个在 `owner == 0` 时把记号丢成
    /// `Name::EMPTY`，而这一手照样报记号——引导期那批设备门闩（owner 0）认的就是它。
    pub fn marked_as(hole) => mark;
}

/// 从**本端**那一枚孔收一句话（有界等）。
pub(super) fn pull_own(hole: PieToken, buf: &mut [u8], millis: usize) -> Result<usize, ()> {
    mail::HolePie::from_token(hole)
        .pull_timeout(buf, millis)
        .map_err(|_| ())
}

/// 等"我自己这张权限表里落进一枚"（`UnitCall::Fall`）。
///
/// 无参数：等的是本端这张表（键由内核从调用者推出来，伪造不出"你的表变了"）。
/// `millis` 属**上限族**（三态口径见 `env::fid` 文件头的定式）。
/// `false` = 自上次取走以来没落过表（期限到）——**醒来自己扫表分辨**。
pub(super) fn fall(millis: usize) -> bool {
    runtime::env::unit::fall(millis).unwrap_or(false)
}

/// 单调时钟读数（纳秒）——有界等待按 deadline 循环用它（不依赖 timebase 频率）。
pub(super) fn now_ns() -> u64 {
    runtime::env::chrono::clock().unwrap_or(0)
}

/// [`core::Quay::unseat`](super::core::Quay::unseat) 过线的那一句话：一个字节。
///
/// **只在自己这一层有意义**——"这条别用了"，不带任何领域语义（帧形与负码表仍归具体
/// 协议）。它是泊位自己的名字牌，不是一种新帧。
pub const UNSEAT: [u8; 1] = [0];

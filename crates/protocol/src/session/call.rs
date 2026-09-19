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

use env::{Name, PieToken, TaskId};

use super::core::Claim;

use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

/// 铸一枚孔（本端那一枚），并把**记号**刻在它上面——记号 = 这条路的名字。
pub(super) fn mint(mark: Name) -> Result<PieToken, ()> {
    mail::unseal_hole(mark.as_str()).map_err(|_| ())
}

/// 交给对端：一枚副本种进它表里，返**种在它表里的那个号**。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此**可以再授出**。那一格不是客气——
/// 内核的 `Accord` 有一道闸是"持 `VEST` 才交得出去"（`Need::Grant`），而"对端把这一枚
/// 转给第三方"是本结构里**必然**发生的一步：子方只认得它的生我者，故它交出来的孔先落在
/// 生我者表里，再由生我者转授（见正文事实 1 的推论；板那条路就是这么接上的）。
/// 不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对——看上去像"板坏了"。
pub(super) fn ship(hole: PieToken, peer: TaskId) -> Result<PieToken, ()> {
    let pie = mail::HolePie::from_token(hole);
    port::ship(&pie, peer, Access::FETCH | Access::STORE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| ())
}

/// 放下本端那一枚孔。
pub(super) fn drop_local(hole: PieToken) -> Result<(), ()> {
    mail::release(hole).map_err(|_| ())
}

/// 往**对端**那一条泊位说一句话（号是**种在它表里**的那一枚）。
pub(super) fn post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    mail::HolePie::from_token(at_peer).push(msg).map_err(|_| ())
}

/// 扫我表里的**每一枚**孔，逐枚交给 `f`。
///
/// **不设上限**：表里可能已经有几十枚（设备门闩、别人给的副本……），而认领只关心其中
/// 的一两枚——设一个"最多看几枚"的缓冲会让排在后面的那枚永远看不见。故这里不攒数组，
/// 只把每一枚交出去；"要不要"由调用方说（正文第 6 条）。
///
/// `Err(Claim::Unread)` = 枚举本身失败（我的表读不动了）。
pub(super) fn each(mut f: impl FnMut(Hole) -> Result<(), Claim>) -> Result<(), Claim> {
    let mut index = 0usize;
    loop {
        let Ok((token, _perm, _grantor)) = mail::collect(index) else {
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

/// 我表里的一项：一枚孔 + **谁开的** + **刻的什么记号**（[`each`] 交出来的那两格）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Hole {
    pub token: PieToken,
    /// 这扇门谁开的（`None` = 查不出出处，如引导期那批设备门闩）。
    pub owner: Option<TaskId>,
    /// 这条路上刻的记号（`Name::EMPTY` = 这一枚不是孔、问不到记号）。
    pub mark: Name,
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
pub(super) fn reserve(hole: PieToken) -> (Option<TaskId>, Name) {
    match mail::reserve(hole) {
        Ok((_vestor, owner, mark)) if owner.get() != 0 => (Some(owner), mark),
        _ => (None, Name::EMPTY),
    }
}

/// 从**本端**那一枚孔收一句话（有界等）。
pub(super) fn pull_own(hole: PieToken, buf: &mut [u8], ms: usize) -> Result<usize, ()> {
    mail::HolePie::from_token(hole)
        .pull_timeout(buf, ms)
        .map_err(|_| ())
}

/// 等"我自己这张权限表里落进一枚"（`UnitCall::Fall`）。
///
/// 无参数：等的是本端这张表（键由内核从调用者推出来，伪造不出"你的表变了"）。
/// `ms` 属**上限族**（三态口径见 `env::fid` 文件头的定式）。
/// `false` = 自上次取走以来没落过表（期限到）——**醒来自己扫表分辨**。
pub(super) fn fall(ms: usize) -> bool {
    runtime::env::unit::fall(ms).unwrap_or(false)
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

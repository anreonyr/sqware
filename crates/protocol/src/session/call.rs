//! session 的转发层 —— 内核那几只手的别名。
//!
//! 本文件**一个判断都没有**：判据只有一条可机械检查的纪律——
//!
//! > `call.rs` 里不出现 `if` / `match` 判断。
//!
//! 全会话的规矩（额度、齐没齐、谁的孔归谁）都在 [`core`](super::core)。这里只做两件事：
//! **转发一次**、把内核的错误码翻成"没成"。

use env::{PieToken, TaskId};

use super::core::Claim;

use runtime::core::port::{self, Access, Policy};
use runtime::core::unit;
use runtime::env::mail;

/// 我是谁（判据"这枚孔**是不是我开的**"用它）。
///
/// 答不出（`SelfId` 的越界哨兵，`TaskId(0)`）时返 `None`：那一刻"我开的孔"判不出来，
/// 认领于是退回到只排除**本端那几条泊位自己的孔**（比认错强）。
pub(super) fn me() -> Option<TaskId> {
    match unit::self_id() {
        Ok(id) if id.get() != 0 => Some(id),
        _ => None,
    }
}

/// 铸一枚孔（本端那一枚）。
pub(super) fn mint() -> Result<PieToken, ()> {
    mail::unseal_hole().map_err(|_| ())
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
    port::ship(&pie, peer, Access::READ | Access::WRITE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| ())
}

/// 放下本端那一枚孔。
pub(super) fn drop_local(hole: PieToken) -> Result<(), ()> {
    mail::release(hole).map_err(|_| ())
}

/// 往**对端**那一条泊位说一句话（号是**种在它表里**的那一枚）。
pub(super) fn post(at_peer: PieToken, msg: &[u8]) -> Result<(), ()> {
    mail::HolePie::from_token(at_peer)
        .push(msg)
        .map_err(|_| ())
}

/// 扫我表里的**每一枚**孔，逐枚交给 `f`。
///
/// **不设上限**：表里可能已经有几十枚（设备门闩、别人给的副本……），而认领只关心其中
/// 的一两枚——设一个"最多看几枚"的缓冲会让排在后面的那枚永远看不见。故这里不攒数组，
/// 只把每一枚交出去；"要不要"由调用方说（正文第 6 条）。
///
/// `Err(Claim::NoPeer)` = 枚举本身失败（我的表读不动了）。
pub(super) fn each(mut f: impl FnMut(Hole) -> Result<(), Claim>) -> Result<(), Claim> {
    let mut index = 0usize;
    loop {
        let Ok((token, _perm, _grantor)) = mail::collect(index) else {
            return Err(Claim::NoPeer);
        };
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return Ok(());
        }
        index += 1;
        f(Hole {
            token,
            owner: owner_of(token),
        })?;
    }
}

/// 我表里的一项：一枚孔 + **谁开的**（[`each`] 交出来的那一格）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Hole {
    pub token: PieToken,
    /// 这扇门谁开的（`None` = 查不出出处，如引导期那批设备门闩）。
    pub owner: Option<TaskId>,
}

/// 这枚孔**是谁开的**（`Reserve` 的 `owner`：副本共享同一事实，转手不变）。
///
/// 认"对端放进来的那一枚"用它：本端 `mint` 出来的 owner 是本端，对端 `ship` 进来的
/// owner 是对端——**编号比不出来，这一格比得出来**。
///
/// `owner` 是 0（引导期那批设备门闩）时报 [`None`]：**查不出出处的不算对端那一枚**。
pub(super) fn owner_of(hole: PieToken) -> Option<TaskId> {
    match mail::reserve(hole) {
        Ok((_vestor, owner)) if owner.get() != 0 => Some(owner),
        _ => None,
    }
}

/// 从**本端**那一枚孔收一句话（有界等）。
pub(super) fn pull_own(hole: PieToken, buf: &mut [u8], ms: usize) -> Result<usize, ()> {
    mail::HolePie::from_token(hole)
        .pull_timeout(buf, ms)
        .map_err(|_| ())
}

/// 让出处理器。认领是**探询**：表里多了一枚孔不是内核的事件，挂不起。
pub(super) fn nap(ms: usize) {
    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(ms as u64));
}

/// 探询的步长（毫秒）。
pub(super) const POLL_MS: usize = 1;

/// [`core::Quay::unseat`](super::core::Quay::unseat) 过线的那一句话：一个字节。
///
/// **只在自己这一层有意义**——"这条别用了"，不带任何领域语义（帧形与负码表仍归具体
/// 协议）。它是泊位自己的名字牌，不是一种新帧。
pub const UNSEAT: [u8; 1] = [0];

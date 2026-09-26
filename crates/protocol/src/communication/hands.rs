//! hands — **内核那几只手的身体**（一个调用一处转发，不做裁决）。
//!
//! 这里的每一手都是**一次转发**：把内核的负码折成"没成"，或把 `Reserve` 的三格摊平。全层的
//! 规矩（谁的孔归谁、认领按哪两格）住 [`super::establish`]；本文件**一格裁决都没有**。
//!
//! # 面为什么这么小
//!
//! 建立那一半（铸、交、认）已经收进 [`super::establish`]（`endpoint` / `give` / `find` /
//! `claim`）；收发那一半收进 [`super::sender`] / [`super::receiver`]。本文件只剩三类**建立那一手
//! 的原语**——它们没有更上层的东西可挂：
//!
//! ```text
//!   ship / unship / lend_out / push_to   交一枚副本 / 放下 / 借一枚回信孔 / 往一扇门推一帧
//!   unseal（本文件内）                    铸一枚孔（记号刻在上面）
//!   vested_by / opened_by / marked_as     Reserve 的三格：谁授的 / 谁开的 / 刻的什么
//! ```
//!
//! **原 `session/call.rs` 里那几手已随它们各自的读者一起退场**：推与收进了两个手柄
//! （`Sender::send` / `Receiver::recv`），扫表与等待进了 `establish` 的 `claim`；而那张函数
//! 指针表（`Hands`）连同"喂一张假表推理"那条路一起撤掉——宿主靶已删，那条路的消费者为零
//! （见 `lib.rs` 的照实记）。
//!
//! # 共享身体的家
//!
//! `Reserve` 三格原先在板 / 树 / 线各抄一份、且各起一个名；现在只住这里，其余模块
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

use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

/// 铸一枚孔（**记号刻在上面**）：对端认领时按"谁开的 ＋ 记号"两格找的就是它。
fn unseal_hole(mark: Mark) -> Result<PieToken, ()> {
    mail::unseal_hole(mark).map_err(|_| ())
}

/// **交出**：一枚副本种进对端表里，返**种在它表里的那个号**。
///
/// 这一族只有三个名字：**`ship` 交出 / `take` 接手 / `unship` 放下**（[`unship`] 是它的反面）。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：对端因此**可以再授出**。那一格不是客气——内核的
/// `Accord` 有一道闸是"持 `VEST` 才交得出去"，而"对端把这一枚转给第三方"是**必然**发生的一步：
/// 子方只认得它的生我者，故它交出来的孔先落在生我者表里，再由生我者转授（板那条路就是这么
/// 接上的）。不给 `VEST` 的症状是**转授那一步答 `Denied`**，而两侧已经配好了对——看上去像
/// "板坏了"。
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

/// 把一帧推上那扇门（**裸字节**：装与解不在这里——调用方自己编）。
pub fn push_to(entry: PieToken, frame: &[u8]) -> Result<(), ()> {
    mail::HolePie::from_token(entry)
        .push(frame)
        .map_err(|_| ())
}

/// **借一枚回信孔过去、但先不推**：返 `(本端那一枚, 对端表里那一枚)`。
///
/// **照实记（用户裁定甲′）**：`port::ship` 的 `to.seed()` 就是"**我给你的那一枚在你表里是几号**"，
/// 而从前那一版（`lend`）把它扔了 ⇒ 收方只能**扫全表**按"谁给的 ＋ 记号"把这一枚认回来。
/// 门上量出来那一扫是**每帧 ~6.5 ms**（表 16 枚 ⇒ O(n²)）。故这一手把第二格交出来，好让它
/// **随帧一起过去**；帧由调用方自己推（[`push_to`]）。
///
/// 次序仍是契约的一半：**先铸、先交**（这一手），**再推**（下一手）。
pub fn lend_out(entry: PieToken, mark: Mark) -> Result<(PieToken, PieToken), ()> {
    let host = opened_by(entry).ok_or(())?;
    let back = unseal_hole(mark)?;
    match port::ship(
        &mail::HolePie::from_token(back),
        host,
        Access::STORE,
        Policy::NONE,
    ) {
        Ok(to) => Ok((back, to.seed())),
        Err(_) => {
            let _ = mail::release(back);
            Err(())
        }
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
    /// **为什么另开一手、而不是折进 [`opened_by`] 那一格**：`opened_by` 在 `owner == 0`
    /// （引导期那批设备门闩）时把整条候选判成"不成立"、连记号一起丢；而"这一枚是不是
    /// `entry`"在 owner 0 的那批门闩上照样要答得出。
    pub fn marked_as(hole) => mark;
}

/// 这一枚孔的两格事实（**一次 `Reserve` 取回**）：**谁开的** + **刻的什么记号**。
///
/// 它只服务"手里已经有一个号、问这两格"那一档（`owner` ＋ `mark` 的正判据）。
///
/// `owner` = 这扇门谁开的：副本共享同一事实，转手不变。`mark` = 铸者刻在孔上的那个名字，
/// 同样随副本过线。认"对端放进来的那一枚"就认这两格：本端铸的 owner 是本端、记号是本端刻的
/// 那个；对端 `ship` 进来的 owner 是对端——**编号比不出来，这两格比得出来**。
pub fn reserve(hole: PieToken) -> (Option<TaskId>, Mark) {
    match mail::reserve(hole) {
        Ok((_vestor, owner, mark)) if owner.get() != 0 => (Some(owner), mark),
        _ => (None, Mark::NONE),
    }
}

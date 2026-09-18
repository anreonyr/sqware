//! board 的转发层 —— 内核那几只手的别名。
//!
//! 本文件**一个判断都没有**：判据只有一条可机械检查的纪律——
//!
//! > `call.rs` 里不出现 `if` / `match` 判断。
//!
//! 板上的规矩（谁能挂、挂哪儿、什么时候扫）全在 [`core`](super::core)。这里只做三件事：
//! **转发一次**、把"不在我表里"翻成 [`None`]、把错误码翻成"没成"。
//!
//! 板服务（板侧待客 / 客侧问一句）住在 `programs/src/bin/supervisor/board.rs`：
//! 本模块只给**两边都要用的那几手**（认来源 / 认出这扇门是谁的 / 授出 / 收下 / 帧）。

use env::{Name, PieToken, TaskId};

use super::core::{Board, Fail, Free, Probe};
use super::desk::Desk;

use crate::session::{Claim, Seat};

use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

/// 活性：这枚入口**还答得出吗**？**谁授给我的**？
///
/// 答的是 `Reserve` 的第一格（`vestor` = 这枚门闩谁授的）。`register` 的判据就是它
/// ——"**那枚入口是你亲手交给我的**"。
///
/// 与 [`opened_by`] 分工：这一格答"谁交来的"，那一格答"这扇门本身是谁的"。转手会改写
/// 这一格（root 转授过的门闩，`vestor` 会变成 root），故**不能用它认"对端是谁"**。
///
/// `reserve` 答"不在我表里"与"令牌越界"是同一个 `Err`。这两桩在今天**同粒度**，不是
/// 因为它们本来就同：句柄是**表**的（`PieToken` 标着 `!Send`，见 `env::wire::handle`），
/// **外来的号进不了这张表**——故剩下的两种情形才都是"实例没了"。板上那一枚又是**亲手
/// 交给板的**（板正是持有它的那张表），所以这里读到 `Err` 就是"实例真的没了"。
///
/// **"实例没了"这一格里就有"那扇门封印了"**：`Reserve` 的 `owner` 那一格带存活闸
/// （内核 `envcall/pie.rs` 的 `owner().ok_or(GateError::Dead)`，闸在
/// `work/unit/gate/pie.rs` 的 `alive().then(...)`）⇒ 开者一退场、它开的门随之封印
/// （`gate/cull.rs` 的退场钩子）⇒ 这里当场答 `Err(-2 Dead)`，与 `env::fid` 的 `Reserve`
/// 注记写着的那条契约一致。**本格因此不必再问第二个问题**：板侧两本账的"死"判据读的
/// 都是它，而封印发生在退出钩子里、**早于叫醒板的那一跳** ⇒ 板被叫醒时这一格已是定论。
fn probe(entry: PieToken) -> Option<TaskId> {
    mail::reserve(entry)
        .ok()
        .map(|(vestor, _owner, _mark)| vestor)
}

/// 这扇门**本身是谁的**（`Reserve` 的第二格 `owner`：副本共享同一事实，转手不变）。
///
/// 板那一侧认孔的两处都读它，且都问同一句——"**是不是这位客人的**"：答话路（客人开的那扇
/// 门）与问话孔（客人铸的那一枚）。分开这两处的是**另外半格**：来源位（谁交给板的）。
///
/// 问不到那两格（这一枚**不是孔**、或它已不在表里）⇒ `None`：**这一条候选不成立**
/// （记号只长在孔上，故 Pole/Nole/Tole 一类问不到 owner）。
pub fn opened_by(hole: PieToken) -> Option<TaskId> {
    match mail::reserve(hole) {
        Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
        _ => None,
    }
}

/// 这条路上刻的**记号**（`Reserve` 的第三格：铸者刻在孔上的那个名字，副本共享同一事实、
/// 转手不变）。
///
/// 板那一侧三处认法都读它，各自问同一句——"**这是哪条路上的那一枚**"：答话路（板路，
/// 记号 = `board`）、问话孔（记号 = `ask`）、注册入口（记号 = `entry`）；把同一来源的两枚
/// 分开的是**另外半格**（谁交的 / 谁开的）。问不到记号（这一枚**不是孔**、或它已不在表里）
/// ⇒ `None`：**这一条候选不成立**。
pub fn mark_of(hole: PieToken) -> Option<Name> {
    match mail::reserve(hole) {
        Ok((_vestor, _owner, mark)) => Some(mark),
        _ => None,
    }
}

/// 放下：自释一份。
fn free(entry: PieToken) -> Result<(), ()> {
    mail::release(entry).map_err(|_| ())
}

/// 立一块板：把两枚机制函数交给核心（核心因此不 `use` 内核）。
///
/// `const` 是为了它能当 `static` 的初值：板只有一份，住在本域（`supervisor/board.rs`）。
pub const fn board() -> Board {
    let probe: Probe = probe;
    let free: Free = free;
    Board::new(probe, free)
}

/// 立一本**板侧的账**（一位客人一格：谁 / 问 / 答）。与 [`board`] 同一个注入（探活那一格）。
///
/// `const` 同理：账只有一本，住在板线程（`supervisor/board.rs`）。
pub const fn desk() -> Desk {
    let probe: Probe = probe;
    Desk::new(probe)
}

/// 挂上：把调用方手里那枚入口**交给持板者**（`Accord` 一份副本），返"种在持板者表里"的号。
///
/// 这就是"谁挂的"的来历：板上那枚是**亲手交出去的**，故 `probe` 认得出谁授的它。
/// 权限给满（`R|W`）**加一格 `VEST`**：入口要能用来说话，而持板者的本职就是**再授出**
/// （`Query` 的下场）——内核那道"持 `VEST` 才交得出去"的闸（`Need::Grant`）挡的就是
/// "板查到了却授不出去"。
pub fn hang_in(entry: PieToken, holder: TaskId) -> Result<PieToken, ()> {
    let pie = mail::HolePie::from_token(entry);
    port::ship(&pie, holder, Access::READ | Access::WRITE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| ())
}

/// 授出：把板上那一份入口转授给调用方（`Query` 的下场）。
///
/// 与 [`hang_in`] 同一份子集（`R|W|VEST`）：**入口可以再传**——拿到它的人把它转给第三方
/// 是常态（那正是"一个名字指向一个入口"的用法），故这里不替调用方裁剪。
pub fn give(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    let pie = mail::HolePie::from_token(entry);
    port::ship(&pie, to, Access::READ | Access::WRITE, Policy::VEST)
        .map(|to| to.seed())
        .map_err(|_| Fail::Denied)
}

/// 牌子上的名字（**定长解码面**：尾随 NUL 是填充，不是内容）。
pub fn name_of(bytes: &[u8]) -> Option<Name> {
    let at = bytes.get(..env::wire::NAME_LEN)?;
    let mut raw = [0u8; env::wire::NAME_LEN];
    raw.copy_from_slice(&at[..]);
    Name::from_bytes(raw).ok()
}

// ── 一问一答那一步（`Query` 的载体）──────────────────────────

/// 一问一答的帧：**一问一答各一句**。
///
/// ```text
///   Query   [0] op   [1..33] name   [33..41] 入口号（只有 Register 用）
///   Short   [0] op                                （Dismiss：整帧一字节，空载荷）
///   Reply   [0] status
/// ```
///
/// 答话那一格的六个码见 [`OK`] / [`UNKNOWN`] / [`TAKEN`] / [`DENIED`] / [`FULL`] / [`BAD`]。
///
/// 名字按 [`NAME_LEN`](env::wire::NAME_LEN) 定长写（尾随 NUL 是填充）——**与牌子同
/// 一个解码面**，故 `Name` 的读法全树只有一处。
///
/// 那一格入口号是**客人把入口交出去之后、换回来的"种在板表里"的号**
/// （[`hang_in`] 的返回值）——不是"客人的入口是几号"。两个编号空间不同源，互相拿错
/// 正是旧树 `[33..41]` 那一格的病；**答案那一侧则干脆没有这一格**：查到的那枚入口经
/// 会话交进客人的表，报文里再放一个号只会多出一份两边都得认的约定。
///
/// 报文的**上限就是 [`ASK`]**：本协议只有两种帧——这一种有载荷的（[`REGISTER`] /
/// [`UNREGISTER`] / [`LOOKUP`]）与 [`DISMISS`] 的一字节短帧。长度仍由每次 `push` 自己带
/// （孔不预设上限），这里只是**声明这一版只用多大**——一处上界。
pub const ASK: usize = 1 + env::wire::NAME_LEN + 8;

/// 四个动作在报文里的码——**与核心那四个方法同名**（`register` / `unregister` /
/// `lookup` / `dismiss`）：线上与模型是同一件事的两层，不该各起一套词。
pub const REGISTER: u8 = 1;
pub const UNREGISTER: u8 = 2;
pub const LOOKUP: u8 = 3;
/// 第四格动作码：**空载荷**——退场那一句没有名字、也没有入口，故整帧只有这一字节
/// （给它塞两格空位就白要 40 字节，见 [`op_of`] 与 [`unpack`] 的分工）。
pub const DISMISS: u8 = 4;

/// 答话那一格。**前五格与 [`Fail`] 一一对应**（`OK` = 一个失败都不是），第六格不是
/// 失败域的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持有者那一侧编、客人那一侧读）。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const FULL: u8 = 4;
pub const BAD: u8 = 5;

/// 把一问编成字节。`seed` 只有 [`REGISTER`] 用得上。
pub fn pack(op: u8, name: Name, seed: Option<PieToken>) -> [u8; ASK] {
    let mut out = [0u8; ASK];
    out[0] = op;
    out[1..1 + env::wire::NAME_LEN].copy_from_slice(name.bytes());
    if let Some(seed) = seed {
        out[1 + env::wire::NAME_LEN..].copy_from_slice(&seed.to_bytes());
    }
    out
}

/// 只读第一格**动作码**——短帧也读得动，故分流先问这一句。空帧 ⇒ `None`
/// （持板者据此答 `BAD`，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问的**载荷**：`(名字, 那一格入口号)`。**读不懂返 `None`**（持板者据此答 `BAD`，
/// 不猜、不崩）。
///
/// 只在**有载荷**的那几码上叫（[`op_of`] 已经分过流：退场那一句是一字节短帧，不进这里）。
/// 长度为 [`ASK`] 是**帧的契约**（`pack` 产出的就是这个长度），故短一字节即读不懂。
/// 那一格入口号按 [`PieToken::NONE`] = "没带"解——令牌自 1 起，0 是内核的越界哨兵。
pub fn unpack(bytes: &[u8]) -> Option<(Name, PieToken)> {
    let name = name_of(bytes.get(1..)?)?;
    let at = bytes.get(1 + env::wire::NAME_LEN..ASK)?;
    Some((name, PieToken::from_bytes(at)?))
}

/// 会话的失败域 → 板的失败域：**"它不在"是一条判据**，故两边只留一个名字
/// （`Fail::Unknown`）。
///
/// 「一笔都没到」与「到了一些、不齐」在上面那一层都归 `Unknown`/`Full`：板这一侧
/// 只有一格答话码，问的人按它决定要不要重问。
pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        // 我的表读不动 ⇒ 这一问没有答案（与"它不在"同一格：都不是"板答了没有"）。
        Claim::Unread => Fail::Unknown,
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

/// 装一条路的失败域 → 板的失败域。
///
/// 与 [`map_claim`] 同一条口径：名字/资源上的毛病（名字非法、同名已装、铸不出孔）是
/// **调用方写错了** ⇒ `Denied`；交不出去（对端已不在）⇒ `Unknown`（"它不在"）；
/// 账腾不出来 ⇒ `Full`。
pub fn map_seat(seat: Seat) -> Fail {
    match seat {
        Seat::NoName => Fail::Denied,
        Seat::NoHole => Fail::Denied,
        Seat::NoSeed => Fail::Unknown,
        Seat::NoRoom => Fail::Full,
    }
}

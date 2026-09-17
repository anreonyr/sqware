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

use crate::session::{Claim, Pier};

use runtime::core::port::{self, Access, Policy};
use runtime::env::mail;

/// 活性：这枚入口还在我的表里吗？**谁授给我的**？
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
fn probe(entry: PieToken) -> Option<TaskId> {
    mail::reserve(entry).ok().map(|(vestor, _owner)| vestor)
}

/// 这扇门**本身是谁的**（`Reserve` 的第二格 `owner`：副本共享同一事实，转手不变）。
///
/// 板那一侧认孔的两处都读它，且都问同一句——"**是不是这位客人的**"：答话路（客人开的那扇
/// 门）与问话孔（客人铸的那一枚）。分开这两处的是**另外半格**：来源位（谁交给板的）。
pub fn opened_by(hole: PieToken) -> Option<TaskId> {
    match mail::reserve(hole) {
        Ok((_vestor, owner)) if owner.get() != 0 => Some(owner),
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

/// 一问一答的帧：**一问一答各一句**，形状只此一种。
///
/// ```text
///   Query   [0] op   [1..33] name   [33..41] 入口号（只有 Register 用）
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
/// 报文的**上限就是 [`ASK`]**：本协议没有第二种消息。长度仍由每次 `push` 自己带
/// （孔不预设上限），这里只是**声明这一版只用多大**——一处上界。
pub const ASK: usize = 1 + env::wire::NAME_LEN + 8;

/// 三个动作在报文里的码——**与核心那三个方法同名**（`register` / `unregister` /
/// `lookup`）：线上与模型是同一件事的两层，不该各起一套词。
pub const REGISTER: u8 = 1;
pub const UNREGISTER: u8 = 2;
pub const LOOKUP: u8 = 3;

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

/// 解开一问：`(动作码, 名字, 那一格入口号)`。**读不懂返 `None`**（持板者据此答 `BAD`，
/// 不猜、不崩）。
///
/// 长度为 [`ASK`] 是**帧的契约**（`pack` 产出的就是这个长度），故短一字节即读不懂。
/// 那一格入口号按 0 = "没带"解——令牌自 1 起，0 是内核的越界哨兵。
pub fn unpack(bytes: &[u8]) -> Option<(u8, Name, Option<PieToken>)> {
    let op = *bytes.first()?;
    let name = name_of(bytes.get(1..)?)?;
    let at = bytes.get(1 + env::wire::NAME_LEN..ASK)?;
    let seed = PieToken::from_bytes(at)?;
    let seed = (seed.get() != 0).then_some(seed);
    Some((op, name, seed))
}

/// 会话的失败域 → 板的失败域：**"它不在"是一条判据**，故两边只留一个名字
/// （`Fail::Unknown`）。
pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        Claim::Nameless => Fail::Denied,
        Claim::NoPeer => Fail::Unknown,
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

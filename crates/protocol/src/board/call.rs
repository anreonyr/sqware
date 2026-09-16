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
//! 本模块只给**两边都要用的那几手**（盖章 / 探活 / 授出 / 放下 / 帧）。

use env::{Name, PieToken, TaskId};

use super::core::{Board, Fail, Free, Probe};

use crate::session::Claim;

use runtime::core::port::{self, Access, Policy};
use runtime::core::unit::self_id;
use runtime::env::mail;

/// 持板者认的调用方身份：**内核在 Push 那一刻盖章**的那个 id。
///
/// 不看报文里任何字段——门闩号是全局连续小整数，猜中别人的号就能冒充。
/// `TaskId(0)` = 没有上下文（内核侧的越界哨兵，与 `UnitCall::SelfId` 同值）。
pub fn me() -> TaskId {
    self_id().unwrap_or(TaskId::new(0))
}

/// 活性：这枚入口还在我的表里吗？谁授的？
///
/// `reserve` 答"不在我表里"与"令牌越界"是同一个 `Err`，**正是牌子要的粒度**——两种
/// 情形都是"实例没了"。
fn probe(entry: PieToken) -> Option<TaskId> {
    mail::reserve(entry).ok().map(|(vestor, _owner)| vestor)
}

/// 放下：自释一份。
fn free(entry: PieToken) -> Result<(), ()> {
    mail::release(entry.get()).map_err(|_| ())
}

/// 立一块板：把两枚机制函数交给核心（核心因此不 `use` 内核）。
pub fn board() -> Board {
    let probe: Probe = probe;
    let free: Free = free;
    Board::new(probe, free)
}

/// 挂上：把调用方手里那枚入口**交给持板者**（`Accord` 一份副本），返"种在持板者表里"的号。
///
/// 这就是"谁挂的"的来历：板上那枚是**亲手交出去的**，故 `probe` 认得出谁授的它。
/// 权限给满（`R|W`）：入口要能用来说话，持板者不替调用方裁剪。
pub fn hang_in(entry: PieToken, holder: TaskId) -> Result<PieToken, ()> {
    let pie = mail::HolePie::from_token(entry.get());
    port::ship(&pie, holder, Access::READ | Access::WRITE, Policy::NONE)
        .map(|to| to.seed())
        .map_err(|_| ())
}

/// 授出：把板上那一份入口转授给调用方（`Query` 的下场）。
pub fn give(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    let pie = mail::HolePie::from_token(entry.get());
    port::ship(&pie, to, Access::READ | Access::WRITE, Policy::NONE)
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

/// 把一问编成字节。`seed` 只有 [`REGISTER`] 用得上。
pub fn pack(op: u8, name: Name, seed: Option<PieToken>) -> [u8; ASK] {
    let mut out = [0u8; ASK];
    out[0] = op;
    out[1..1 + env::wire::NAME_LEN].copy_from_slice(name.bytes());
    if let Some(seed) = seed {
        out[1 + env::wire::NAME_LEN..].copy_from_slice(&(seed.get() as u64).to_le_bytes());
    }
    out
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

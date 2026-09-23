//! 语义句柄——envcall 载荷里的**类型化身份**。
//!
//! 每个句柄都是 `usize` 的 newtype（`#[repr(transparent)]`），并且：
//!   - `Wire` impl 负责与寄存器宽度往返（本模块内）；
//!   - `From<_> for usize` 供内核侧直接取裸值。
//!
//! 与裸 `usize` 的区别只在类型层：把「第几个参数是什么 id」从注释约定变成
//! 编译期义务（拿 TaskId 当 TeamId 用在编译期即被拦）。

use core::marker::PhantomData;

use super::{Decode, Wire};

// ── 语义句柄 ────────────────────────────────────────────────────────────

/// per-pie 全局唯一句柄（usize；0 = 无效哨兵）。
///
/// **号全局唯一，却只在"持有它的那张表"里解析得动**：内核按调用方自己的表找它
/// （`pies.iter().find(|p| p.token() == token)`），不在那张表里＝用不动。故这一枚
/// **是表的，不是线程的**——同一个域里两枚线程各有一张表，给同域的线程交一枚孔，
/// 照样要走 `Ship`（session 事实 8）。
///
/// `PhantomData<*const ()>` 是这条规矩的**机制**，不是装饰：它让本类型 `!Send + !Sync`。于是
///
///   - 跨线程共享的账（`static`、`Arc`、`SpinLock<T>`）都要求 `Send`/`Sync` ⇒ **句柄装不进去**；
///   - 产线程的闭包要求 `Send`、其结果也要求 `Send`（`runtime::core::unit::closure`）
///     ⇒ **捕获不进别的线程，也交不回来**。
///
/// 跨线程只剩一条合法的路：`ship`——显式 `.get()` 变裸值过线，对端在自己的表里另落一枚。
/// 要绕过这一层必须**写出**那个 `.get()`：一个可 grep 的动作，不再是顺手存进去。
///
/// 与 [`TaskId`]／[`TeamId`] 成对偶：那两个是**全局身份**（谁都能提谁，可跨线程），这一枚是
/// **表的身份**。注意别把 [`VirtAddr`] 也归进来——那是**空间**的身份，同一个空间里的多枚线程
/// 共用它，跨线程合法；判它在不在自己手里的是空间，不是表。
///
/// **造号的两扇门**：[`PieToken::from_bytes`]（**收号**——号从线上/账上到来）与
/// [`PieToken::mint`]（**铸号**——给"号的来源"那一侧：内核）。用户态那一面仍然只该经
/// `from_bytes` 收号；[`new`] 是 `pub(crate)` 的解码面。
///
/// **照实记**：`mint` 不动能力面——`from_bytes` 本来就公开，用户态今天就能把任意数字拼
/// 成号；而伪造的号在**调用方自己那张表**里查不到（`gate::locate` 只查表）⇒ 照旧
/// `Denied`。两扇门的分工因此是**词汇**的分工："我收到一枚号" vs "我是这枚号的来源"。
///
/// [`from_bytes`]: PieToken::from_bytes
/// [`mint`]: PieToken::mint
/// [`new`]: PieToken::new
/// [`get`]: PieToken::get
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct PieToken(pub usize, PhantomData<*const ()>);

impl PieToken {
    /// 无效哨兵（0）：内核各处"越界不报错"的既有约定（`Reserve` 的 `TaskId(0)` 同理）。
    ///
    /// 命名是 `NONE` 而不是"零"：读的人关心的是"**这枚不是号**"，不是那一位的值。
    pub const NONE: Self = Self(0, PhantomData);

    /// 号在**线上**的宽度（8 字节 LE）：`Sign.entry`、配对块那一格、`Query` 那一格都用它。
    pub const WIDTH: usize = 8;

    /// **造号**——只给本 crate 的解码面（`Wire::unpack` / `FromPair`）用。
    ///
    /// 外面没有这扇门：号只能**到来**（[`PieToken::from_bytes`]）或**交出**（[`PieToken::get`]）。
    pub(crate) const fn new(v: usize) -> Self {
        Self(v, PhantomData)
    }

    /// **铸号**：号的来源那一侧（内核）的门——这枚号的出生地。
    ///
    /// 用户态那一面没有这扇门的**位置**（它只该经 [`PieToken::from_bytes`] 收号）；
    /// 这不是一扇新的能力门，理由见本类型头注的照实记。
    pub const fn mint(v: usize) -> Self {
        Self(v, PhantomData)
    }

    /// **收号**：号从线上/账上到来时解开它。
    ///
    /// 长度不足即读不懂（→ `None`）。这里的 `0` 不作废——它是无效哨兵，由**帧自己的
    /// 约定**处置（如 `Query` 的"没带入口"），故本函数不替调用方筛。
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let at = bytes.get(..Self::WIDTH)?;
        let raw = u64::from_le_bytes(at.try_into().ok()?);
        Some(Self::new(raw as usize))
    }

    /// **交出**：号变回线上的 8 字节（它的对偶面是 [`PieToken::from_bytes`]）。
    pub fn to_bytes(self) -> [u8; Self::WIDTH] {
        (self.0 as u64).to_le_bytes()
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<PieToken> for usize {
    fn from(t: PieToken) -> Self {
        t.0
    }
}

impl Wire for PieToken {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(PieToken::new(v))
    }
}

/// 任务句柄（全局唯一 id；0 = 无上下文）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct TaskId(pub usize);

impl TaskId {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<TaskId> for usize {
    fn from(t: TaskId) -> Self {
        t.0
    }
}

impl Wire for TaskId {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(TaskId(v))
    }
}

/// 用户虚拟地址 / 窗口指针（envcall 侧仅作 ABI 语义标记；内核侧以 `VirtAddr::from_raw`
/// 转入具体地址语义）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct VirtAddr(pub usize);

impl VirtAddr {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        v.0
    }
}

impl Wire for VirtAddr {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(VirtAddr(v))
    }
}

/// 记号：一枚**不透明**的 8 字节键 —— 内核只保管、只递回；**从不解释、从不比较**。
///
/// 与 [`PieToken`] 的分工：`PieToken` 答"**哪一枚**"（唯一、表内坐标），`Mark` 答"**干什么用的**"
/// （可以不唯一，只在"这张表 + 谁开的"里成立）。与 [`Name`](super::Name) 的分工：名字是
/// **本端的账**（泊位名，文本、人读、不过线），记号是**过线的钥匙**（双方约定、只比较）
/// ——两者可以不同（提示孔那一路：本端泊位叫 `operator-tip`，孔上刻的是 `tip`）。
///
/// **源码里仍写着名字**：协议各处用 [`Mark::of`] 从字面串算出来（`Mark::of("board-ask")`），
/// 于是"记号叫什么"在源码里一眼可读，线上只是一枚数。
///
/// **照实记**：`of` 是 64 位 FNV-1a —— 两个不同的名字理论上可能撞成同一枚数，撞了是
/// **静默**的（同一张表里认错孔）。名字总数 ≤ 数十条、空间 2^64，概率 ~1e-17，本仓接受；
/// 要绝对无撞就得改成手排号表（每个协议自己排号，代价是"记号"这个概念要集中到一处，
/// 而它今天住在各协议自己的 `call.rs` 里）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Mark(u64);

impl Mark {
    /// 无效哨兵（0）：与 [`PieToken::NONE`] 同一条约定——读的人关心的是"**这枚不是记号**"。
    pub const NONE: Mark = Mark(0);

    /// 由裸值造一枚（线上解码面）。
    pub const fn new(raw: u64) -> Mark {
        Mark(raw)
    }

    /// 裸值。
    pub const fn get(self) -> u64 {
        self.0
    }

    /// 由**名字**算出来（源码里仍写着名字）：64 位 FNV-1a，两侧各算同一个数。
    pub const fn of(name: &str) -> Mark {
        let bytes = name.as_bytes();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut i = 0;
        while i < bytes.len() {
            h ^= bytes[i] as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
            i += 1;
        }
        Mark(h)
    }

    /// 线上的那一格（8 字节小端）。
    pub const fn to_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    /// 由线上字节还原。
    pub const fn from_bytes(bytes: [u8; 8]) -> Mark {
        Mark(u64::from_le_bytes(bytes))
    }
}

impl Wire for Mark {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0 as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(Mark(v as u64))
    }
}

/// 团队句柄（域标识；0 = 无效哨兵）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct TeamId(pub usize);

impl TeamId {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<TeamId> for usize {
    fn from(t: TeamId) -> Self {
        t.0
    }
}

impl Wire for TeamId {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(TeamId(v))
    }
}

//! Pie 域（class 7）—— **权柄轴**：许可的生死与流动。
//!
//! 与 [`mail`](super::mail)（class 5，**数据轴**：消息穿孔）的分界就是 `env::fid` 文件头
//! 立的那两条正交轴：本类**不搬运载荷**，传的是许可；数据走那边。故两条轴一文件一条，
//! 与其余七个 class 同形（[本层](crate::env) 的口径："每个调用域一个子模块"）。
//!
//! 本文件 = **裸函数层 + 类型化句柄**：每个函数封一次 envcall，零业务逻辑
//! （业务在协议层：谁把哪一枚交给谁、收窄到什么程度）。
//!
//! **四枚句柄的类型都不在这里**——Hole / Nole / Pole / Tole 全住 `mail.rs`（句柄一处）；
//! 本文件只持有 [`AnyPie`] 与它那**四份** `impl`（权柄动词一处）。四种资源的权柄操作
//! 同构，集中在这里各写一遍，而不是散进四个文件里。
//!
//! **名字照旧**：`mail.rs` 把本文件每一项都 `pub use` 转出去，故 `runtime::env::mail::X`
//! 这一形（`protocol` / `programs` / `harness` 里十几处调用点）**一行没改**——与
//! `Access`/`Policy`、`Announce`/`Grant`/`Died` 同一条先例。

use env::{Mark, Permission, PieResult, PieToken, TaskId};

use super::mail::{HolePie, NolePie, PolePie, TolePie};

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 解封 Hole：孔上刻**一格记号**（`mark` = 这条路的名字）。
///
/// **界只有一条，落在载体上**：一条消息 ≤ **一页**（契约在 `env::fid` 的 `Push`）；孔本身
/// 不因此多带参数，也不预分配槽——多出来的只有记号：它随副本过线、转手不变，故"同一位开的
/// 多枚孔"分辨得出（读它走 [`reserve`]）。
///
/// 名字非法（空 / 含 NUL / ≥ 32 字节 / 非 UTF-8）或那段字节拷不动 ⇒ `Denied`。
pub fn unseal_hole(mark: Mark) -> PieResult<PieToken> {
    env::pie::unseal_hole(mark)
}

pub fn unseal_pole(size: usize) -> PieResult<PieToken> {
    env::pie::unseal_pole(size)
}

/// 解封 Nole（**无数据面**的权柄载体）：造一枚只有身份与存活的许可载体。
///
/// **无参数**——没有 mtu、没有字节数。它承载**无载荷通信**（门铃，见
/// [`crate::core::bell`]），与资源权（"你对这份资源能做什么"）正交。
pub fn unseal_nole() -> PieResult<PieToken> {
    env::pie::unseal_nole()
}

/// 开闩：借映 Pole 页进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
///
/// **两件一起返**：起点与长度是同一段区间的两半，而长度只在内核手里（外来区按
/// 页界撑开，设备树 `reg` 声明的长度内核不知道）。
pub fn open(token: PieToken) -> PieResult<(usize, usize)> {
    env::pie::open(token).map(|(va, size)| (va.get(), size))
}

pub fn shut(token: PieToken) -> PieResult<()> {
    env::pie::shut(token)
}

pub fn seal(token: PieToken) -> PieResult<()> {
    env::pie::seal(token)
}

/// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）。
///
/// 返回的是句柄而非裸数：它要经线形送到对方、再由对方 `from_token` 重建——
/// 全程一个 `PieToken`，中途不化成 `usize` 便不会与别的 id 混。
pub fn accord(src: PieToken, dst: TaskId, subset: Permission) -> PieResult<PieToken> {
    env::pie::accord(src, dst, subset)
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: PieToken, subset: Permission) -> PieResult<()> {
    env::pie::narrow(token, subset)
}

/// 收回我授给 `dst` 的副本（含其全部后代）。
///
/// `at_dst` = 该副本在**对端表里**的句柄（[`accord`] 的返回值，经线形送达）——
/// **不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」＝「它是我授出的」。
pub fn revoke(dst: TaskId, at_dst: PieToken) -> PieResult<()> {
    env::pie::revoke(dst, at_dst)
}

/// 表里的一枚（[`collect`] 收拢出来的那一格）——**三件事实一起**，故不必再问第二次。
///
/// 字段名就是判据：`owner` 是**资源**的来历（副本共享同一事实），与"谁授的"（`vestor`，
/// 转手即改写）**不是一回事**——那一格这一手不答（见下），要问就走 [`reserve`]。
///
/// 两处哨兵与 `Reserve` 同一条口径：`TaskId(0)` = 这一格没有答案（原初自持 / 已封印 /
/// 不是孔），[`Mark::NONE`] = 记号那一格没有答案（记号只长在孔上）。
#[derive(Clone, Copy)]
pub struct Pie {
    /// 这一枚在本任务表里的号（[`unseal_hole`] 那一族铸的）。
    pub token: PieToken,
    /// **这扇门谁开的**（副本共享同一事实）；`0` = 查不出（已封印 / 不是孔）。
    pub owner: TaskId,
    /// **这条路的名字**（[`unseal_hole`] 刻的那一格）；`NONE` = 这一枚不是孔。
    pub mark: Mark,
}

/// 收拢：本任务权限表第 `index` 份。越界 → 四格全哨兵（见 [`Pie`]），**不报错**。
///
/// **唯一的枚举手段**（[`reserve`] 是它的对偶：一个按位置问、一个按句柄问）。
/// 一次调用答四格，故"扫一遍这张表"**不必每一枚再问一次 `reserve`**——那一问是一次
/// envcall（~55 µs），表 16 枚 ⇒ 一趟扫描 6.5 ms（读数见
/// `programs/src/driver/rtc/main.rs` 与 `444d1f3`）。
///
/// 原始自持 pie（vestor = None）编码为 `TaskId(0)`，与 `UnitCall::SelfId` 的"无上下文
/// 也是 0"是**同一条哨兵口径**（0 = 这一格没有答案）。
///
/// **不返 `EnvResult`**：内核那一格恒写三件事实，没有失败支（理由见 [`Pies`] 的那条裁定）。
pub fn collect(index: usize) -> Pie {
    let (token, owner, mark) = env::pie::collect(index);
    Pie { token, owner, mark }
}

/// [`collect`] 那条枚举：**0 起、哨兵收尾、越界不报错**——这句话**只写这一处**。
///
/// **照实记（这是"手搓游标"那一刀收出来的）**：先前九处调用点各自把这套仪式抄一遍
/// （起 0 → `collect(i)` → 查哨兵 → `index += 1`），哨兵有两种拼法、`Err` 有三种处置，
/// 而"哪一种对"在源码里没有一处定义。收成迭代器之后，九处只剩"要找什么"。
///
/// **名字照实记**：我起初把它叫 `holes()`——**那个名字是错的**：`Collect` 枚举的是整张
/// 权限表（孔 / 铃 / 页 / 组 / 别人给的副本都在里面），不只是孔。故叫 [`pies`]。
///
/// **每一枚交出去的是三件事实**（[`Pie`]）：token / owner / 记号。这是 `Collect` 宽返回的
/// 唯一用家——从前那三格（token / permission / vestor）里，**扫表的人都还要再问一次 `reserve`**
/// 才拿得到 owner 与记号，而那一问是每一枚一次 envcall。
///
/// **照实记（`vestor` 进过又出去了）**：加宽那一刀把它也带了进来（它当时有两个读者），
/// 而算它要 `gate::vestor(.., &gate::snap())`——**每枚一次全世界快照 ＋ 一次分配**，于是扫表
/// 仍是超线性的（实测 3.36 ms / 16 枚）。乙′ 把那些读者逐个改成"号随交接一起走"（板那一面、
/// 树那一面、两处 `take`），这一格便没有读者、随之撤掉。
///
/// **`Err` 那一格就此打住**（用户裁定甲）：调用方**无从分辨**"表读不动"与"这一遍扫完"。
/// 代价照实记两条：① 约那一侧的 `Claim::Unread`（第三个变体）随之退场；② 另外三处
/// （`operator/server.rs::claim` 与两处 `client.rs::take`）原先"读不动就整趟作废"的
/// fail-closed 一起松掉——`Err` 之前已经认到的那一枚**照旧交出去**。
///
/// **签名即那条裁定**：`collect` 因此返 [`Pie`] 而不是 `EnvResult<Pie>`——内核那一格
/// 恒写三件事实（越界也是四格哨兵），没有失败支；`Pies::next` 先前那一支 `Err` 是死代码。
pub struct Pies {
    index: usize,
    done: bool,
}

impl Iterator for Pies {
    type Item = Pie;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let one = collect(self.index);
        // 越界哨兵：这一遍扫完了（`Collect` 契约：不报错）。
        if one.token == PieToken::NONE {
            self.done = true;
            None
        } else {
            self.index += 1;
            Some(one)
        }
    }
}

/// 我这张权限表里的每一枚（[`collect`] 的 0 起枚举，哨兵收尾）。
pub fn pies() -> Pies {
    Pies {
        index: 0,
        done: false,
    }
}

/// 本端这张权限表里现在有几枚门闩（[`pies`] 数一遍）。
///
/// **给人看的读数，不是给判据用的机制**：它自己不改任何东西。用途只有一个——把"该放下的
/// 放了没有"变成**可量**的一格（少放一枚，这一格当场大 1，见
/// `programs/src/driver/router/main.rs` 的 `drop_lane` 与 `harness/src/lodger/main.rs`）。
pub fn table_size() -> usize {
    pies().count()
}

/// 查询：我持有的这枚门闩——`(vestor, owner, 记号)`。
///
/// `vestor` = 这枚门闩谁授的（转手即改写）；`owner` = 这扇门谁开的（副本共享同一
/// 事实）；**记号** = **这条路的名字**（[`unseal_hole`] 刻的那一格，副本共享同一
/// 事实）。求「对端是谁」一律用 `owner`：root 转发过的门闩，`vestor` 会变成 root。
///
/// 记号收在**栈上 [`NAME_LEN`](env::NAME_LEN) 字节**的缓冲里（不分配），由同一次调用
/// **一格返回**（不再有"先问长度、再备缓冲"那一趟）。这一枚不是孔（记号只长在孔上）、
/// 或表里没有它 ⇒ `Denied`；**资源已封印 ⇒ `Dead`(-2)**——`owner` 那一格带存活闸
/// （见 `env::fid` 的 `Reserve`），故"这一枚答不出"有两个码，别只接 `Denied`。
pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
    // 打包见 `env::fid` 的 `Reserve`：`a0` = owner 高半 | vestor 低半，`a1` = 记号。
    env::pie::reserve(token).map(|(pair, mark)| {
        (
            TaskId::new(pair & 0xffff_ffff),
            TaskId::new(pair >> 32),
            Mark::new(mark as u64),
        )
    })
}

/// 放下：自释本任务的一份门闩（Pole 同步 unmap）。表里无此 token → -1。
pub fn release(token: PieToken) -> PieResult<()> {
    env::pie::release(token)
}

// ── 类型化句柄：**权柄面**（构造 + 种类无关那几手）──

/// 权柄句柄 —— Hole 与 Pole 的**权柄操作同构**，故只写一遍。
///
/// 方法集 = `PieCall` 里「实例作用域 ∧ 与资源种类无关」那一类，一一对应，不多不少：
/// `Seal` / `Narrow` / `Accord` / `Revoke` / `Release`。
///
/// 不在本 trait 的，各有其理由：
/// - **构造**：`unseal*` 产出 `Self`，做不成 `&self` 方法
/// - **任务作用域**：`Collect`（按 index 枚举我表里的）、`Reserve`（按句柄查来历）
///   ——它们不作用在「某一个句柄」上
/// - **资源专属**：`Open`/`Shut`（Pole）、`Push`/`Pull`/`Wait`（Hole）
/// - **表示层转换**：`from_token` / `token` —— 与 ABI 无关
///
/// 镜像内核侧 `gate::AnyPie`（`enum { Hole, Pole, Nole, Tole }`，提供同一批跨种类方法）
/// ——**四个变体、四份 `impl`**：同一条「权柄操作与资源种类无关」的知识在两侧各落一次，
/// 而不是按资源种类各散一份。
///
/// **四份 `impl` 都住本文件**（连 [`HolePie`] 那一份——它的类型定义在 `mail.rs`）：
/// 同构的四份摆在一处才看得出"同构"，散进 `mail.rs` 就只剩四次重复。
pub trait AnyPie {
    /// 封印资源（**只有资源开辟者**可做）。
    ///
    /// 只置死并唤醒等待者，**不摘表项**——持有者仍须 [`release`](AnyPie::release)
    /// 收尾，否则表项泄漏。故 `release` 与 `PolePie::shut` 是**仅有的两处**不过存活闸
    /// 的操作（ABI 那一侧的两条注记同时写着这一条：`env::fid` 的 `Release` / `Shut`）。
    fn seal(&self) -> PieResult<()>;

    /// 收窄本 pie 权限（就地改写，单调；`subset` ⊆ 当前权限）。
    ///
    /// Pole 多一条约束：`subset` 须含 FETCH（RISC-V PTE 无 R=0 的合法数据叶子），
    /// 且会同步把已映射段降权。Hole 无映射，故无此约束。
    fn narrow(&self, subset: Permission) -> PieResult<()>;

    /// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）——
    /// 对方用 `from_token(at_dst)` 重建。
    fn accord(&self, dst: TaskId, subset: Permission) -> PieResult<PieToken>;

    /// 收回我授给 `dst` 的副本（含其全部后代，幂等）。
    ///
    /// `at_dst` = 该副本在**对端表里**的句柄（[`accord`](AnyPie::accord) 的返回值，
    /// 经线形送达）——**不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」。
    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()>;

    /// 放下我这一份（含其全部后代；Pole 同步撤映射）。资源本身不动——封印用
    /// [`seal`](AnyPie::seal)。不需要任何权限位。
    fn release(&self) -> PieResult<()>;
}

// ── 四份同构的实现 ────────────────────────────────────────────────────────
//
// 四个类型都住 `mail.rs`（句柄一处），四份实现都住这里（权柄动词一处）。每一份都只是把
// `self.token()` 递给本文件那几个裸函数——**没有一份多一行**：那正是 trait 头注那句
// "权柄操作与资源种类无关"的读法（`revoke` 的那一格收的是**对端**的句柄，故四份都不看
// `self.token()`，那一处不算例外）。

impl AnyPie for HolePie {
    fn seal(&self) -> PieResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> PieResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        release(self.token())
    }
}

impl AnyPie for NolePie {
    fn seal(&self) -> PieResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> PieResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        release(self.token())
    }
}

impl AnyPie for PolePie {
    fn seal(&self) -> PieResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> PieResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        release(self.token())
    }
}

/// 组（Tole）的那一份——与前三份逐字同构。它成立的前提在内核侧：`gate::AnyPie` 的
/// `Tole` 变体在 `seal` / `narrow` / `accord` / `revoke` / `release` 五条路上都有人接
/// （`envcall/pie.rs`、`gate/narrow.rs`、`gate/accord.rs`、`gate/cull.rs`）——
/// 少一条，这一份就是假接口。
///
/// 共享组（`ToleCall::Unseal { shared: true }`）本来就要经 `accord` 才到得了多个任务
/// （见 `env::fid` 那一格："共享组若不可复制，'多个使用者'是空话"），故这一份不是补上
/// 去的摆设。
impl AnyPie for TolePie {
    fn seal(&self) -> PieResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> PieResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        release(self.token())
    }
}

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

use env::{EnvResult, Mark, Permission, PieCall, PieCallRet, PieToken, TaskId};

use super::mail::{HolePie, NolePie, PolePie, TolePie};

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 解封 Hole：孔上刻**一格记号**（`mark` = 这条路的名字）。
///
/// **界只有一条，落在载体上**：一条消息 ≤ **一页**（契约在 `env::fid` 的 `Push`）；孔本身
/// 不因此多带参数，也不预分配槽——多出来的只有记号：它随副本过线、转手不变，故"同一位开的
/// 多枚孔"分辨得出（读它走 [`reserve`]）。
///
/// 名字非法（空 / 含 NUL / ≥ 32 字节 / 非 UTF-8）或那段字节拷不动 ⇒ `Denied`。
pub fn unseal_hole(mark: Mark) -> EnvResult<PieToken> {
    let r = PieCall::UnsealHole { mark }.call()?;
    match r {
        PieCallRet::UnsealHole(tk) => Ok(tk),
        _ => unreachable!(),
    }
}

pub fn unseal_pole(size: usize) -> EnvResult<PieToken> {
    let r = PieCall::UnsealPole { size }.call()?;
    match r {
        PieCallRet::UnsealPole(tk) => Ok(tk),
        _ => unreachable!(),
    }
}

/// 解封 Nole（**无数据面**的权柄载体）：造一枚只有身份与存活的许可载体。
///
/// **无参数**——没有 mtu、没有字节数。它承载**无载荷通信**（门铃，见
/// [`crate::core::bell`]），与资源权（"你对这份资源能做什么"）正交。
pub fn unseal_nole() -> EnvResult<PieToken> {
    let r = PieCall::UnsealNole.call()?;
    match r {
        PieCallRet::UnsealNole(tk) => Ok(tk),
        _ => unreachable!(),
    }
}

/// 开闩：借映 Pole 页进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
///
/// **两件一起返**：起点与长度是同一段区间的两半，而长度只在内核手里（外来区按
/// 页界撑开，设备树 `reg` 声明的长度内核不知道）。
pub fn open(token: PieToken) -> EnvResult<(usize, usize)> {
    let r = PieCall::Open { token }.call()?;
    match r {
        PieCallRet::Open((va, size)) => Ok((va.get(), size)),
        _ => unreachable!(),
    }
}

pub fn shut(token: PieToken) -> EnvResult<()> {
    let r = PieCall::Shut { token }.call()?;
    match r {
        PieCallRet::Shut(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn seal(token: PieToken) -> EnvResult<()> {
    let r = PieCall::Seal { token }.call()?;
    match r {
        PieCallRet::Seal(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）。
///
/// 返回的是句柄而非裸数：它要经线形送到对方、再由对方 `from_token` 重建——
/// 全程一个 `PieToken`，中途不化成 `usize` 便不会与别的 id 混。
pub fn accord(src: PieToken, dst: TaskId, subset: Permission) -> EnvResult<PieToken> {
    let r = PieCall::Accord { src, dst, subset }.call()?;
    match r {
        PieCallRet::Accord(tk) => Ok(tk),
        _ => unreachable!(),
    }
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: PieToken, subset: Permission) -> EnvResult<()> {
    let r = PieCall::Narrow { token, subset }.call()?;
    match r {
        PieCallRet::Narrow(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收回我授给 `dst` 的副本（含其全部后代）。
///
/// `at_dst` = 该副本在**对端表里**的句柄（[`accord`] 的返回值，经线形送达）——
/// **不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」＝「它是我授出的」。
pub fn revoke(dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
    let r = PieCall::Revoke { dst, token: at_dst }.call()?;
    match r {
        PieCallRet::Revoke(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收拢：本任务权限表第 `index` 份（token + permission + vestor）。
/// 越界 → `(0, 空权限, 0)`——哨兵不报错。
///
/// **唯一的枚举手段**：`protocol::startup::moor()` 靠它发现「父域授给我的那枚门闩」。
/// 已知句柄求事实用 [`reserve`]；原始自持 pie（vestor = None）编码为 `TaskId(0)`，
/// 与 `UnitCall::SelfId` 的"无上下文也是 0"是**同一条哨兵口径**（0 = 这一格没有答案）。
pub fn collect(index: usize) -> EnvResult<(PieToken, Permission, TaskId)> {
    let r = PieCall::Collect { index }.call()?;
    match r {
        PieCallRet::Collect(r) => Ok(r),
        _ => unreachable!(),
    }
}

/// 本端这张权限表里现在有几枚门闩（[`collect`] 一路走到越界哨兵）。
///
/// **给人看的读数，不是给判据用的机制**：它自己不改任何东西。用途只有一个——把"该放下的
/// 放了没有"变成**可量**的一格（少放一枚，这一格当场大 1，见
/// `programs/src/driver/router/main.rs` 的 `drop_lane` 与 `harness/src/lodger/main.rs`）。
pub fn table_size() -> usize {
    let mut n = 0usize;
    loop {
        let Ok((token, _perm, _vestor)) = collect(n) else {
            return n;
        };
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return n;
        }
        n += 1;
    }
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
pub fn reserve(token: PieToken) -> EnvResult<(TaskId, TaskId, Mark)> {
    let r = PieCall::Reserve { token }.call()?;
    match r {
        // 打包见 `env::fid` 的 `Reserve`：`a0` = owner 高半 | vestor 低半，`a1` = 记号。
        PieCallRet::Reserve((pair, mark)) => Ok((
            TaskId::new(pair & 0xffff_ffff),
            TaskId::new(pair >> 32),
            Mark::new(mark as u64),
        )),
        _ => unreachable!(),
    }
}

/// 放下：自释本任务的一份门闩（Pole 同步 unmap）。表里无此 token → -1。
pub fn release(token: PieToken) -> EnvResult<()> {
    let r = PieCall::Release { token }.call()?;
    match r {
        PieCallRet::Release(()) => Ok(()),
        _ => unreachable!(),
    }
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
    fn seal(&self) -> EnvResult<()>;

    /// 收窄本 pie 权限（就地改写，单调；`subset` ⊆ 当前权限）。
    ///
    /// Pole 多一条约束：`subset` 须含 FETCH（RISC-V PTE 无 R=0 的合法数据叶子），
    /// 且会同步把已映射段降权。Hole 无映射，故无此约束。
    fn narrow(&self, subset: Permission) -> EnvResult<()>;

    /// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）——
    /// 对方用 `from_token(at_dst)` 重建。
    fn accord(&self, dst: TaskId, subset: Permission) -> EnvResult<PieToken>;

    /// 收回我授给 `dst` 的副本（含其全部后代，幂等）。
    ///
    /// `at_dst` = 该副本在**对端表里**的句柄（[`accord`](AnyPie::accord) 的返回值，
    /// 经线形送达）——**不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」。
    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()>;

    /// 放下我这一份（含其全部后代；Pole 同步撤映射）。资源本身不动——封印用
    /// [`seal`](AnyPie::seal)。不需要任何权限位。
    fn release(&self) -> EnvResult<()>;
}

// ── 四份同构的实现 ────────────────────────────────────────────────────────
//
// 四个类型都住 `mail.rs`（句柄一处），四份实现都住这里（权柄动词一处）。每一份都只是把
// `self.token()` 递给本文件那几个裸函数——**没有一份多一行**：那正是 trait 头注那句
// "权柄操作与资源种类无关"的读法（`revoke` 的那一格收的是**对端**的句柄，故四份都不看
// `self.token()`，那一处不算例外）。

impl AnyPie for HolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> EnvResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> EnvResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token())
    }
}

impl AnyPie for NolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> EnvResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> EnvResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token())
    }
}

impl AnyPie for PolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> EnvResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> EnvResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
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
    fn seal(&self) -> EnvResult<()> {
        seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> EnvResult<()> {
        narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission) -> EnvResult<PieToken> {
        accord(self.token(), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token())
    }
}

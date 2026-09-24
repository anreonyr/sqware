//! Mail 域：门闩操作。
//!
//! **两条轴**：权柄（`PieCall`，class 7）走 `unseal_*`/`open`/`shut`/`seal`/`accord`
//! /`narrow`/`revoke`/`collect`/`reserve`/`release`；数据（`MailCall`，class 5）走
//! `push`/`pull`/`wait`。本模块是两者的**裸函数层**——每个函数封一次 envcall，
//! 零业务逻辑。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。
//! 用户句柄统一为 per-pie `token`（全局唯一，usize）。
//!
//! 方案 3（typed payload）：`PieCall::X{ .. }.call()?` 直接得 `PieCallRet`，
//! 参数在构造时类型安全（PieToken/VirtAddr/TaskId/Permission），返回值经 from_pair
//! 蒸馏为 Ret 载荷。裸函数层只封 Ret、零业务逻辑。
//!
//! push/pull 的阻塞：内核 Push/Pull 槽满/槽空返 `-3 Busy`；本层转 `Wait` 原语
//! 挂起（让出 CPU），被对侧唤醒后重试——真阻塞，不占核。

use env::{
    EnvResult, HoleDir, MailCall, MailCallRet, Mark, PieCall, PieCallRet, PieToken, TaskId,
    VirtAddr,
};

/// 单调时钟读数（纳秒）——`pull_timeout` 的 deadline 用（机器无关，不依赖
/// timebase 频率）。
fn now_ns() -> EnvResult<u64> {
    crate::env::chrono::clock()
}

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 解封 Hole：孔上刻**一格记号**（`mark` = 这条路的名字）。
///
/// **消息本身仍不预设上限**（那是协议自己的事），也不预分配槽——多出来的只有记号：
/// 它随副本过线、转手不变，故"同一位开的多枚孔"分辨得出（读它走 [`reserve`]）。
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

/// push 一条消息（`msg[..len]` 进 hole 槽）。`len ≥ 1`（**无上限**）。
pub fn push(token: PieToken, msg: *const u8, len: usize) -> EnvResult<()> {
    let r = MailCall::Push {
        token: token,
        msg: VirtAddr::new(msg as usize),
        len,
    }
    .call()?;
    match r {
        MailCallRet::Push(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// pull 一条消息（最多装 `buf[..max]`）。返实际长度（≤ max）；发送者丢弃。
/// 装不下返 `Denied` 且槽原样；要问长度用 [`pull_len`]。
pub fn pull(token: PieToken, buf: *mut u8, max: usize) -> EnvResult<usize> {
    pull_from(token, buf, max).map(|(n, _)| n)
}

/// 只问长度（**不动槽**）：返槽里那条消息的长度与发送者，一个字节都不取。
///
/// 走 `Pull { max: 0 }`——与 `Wait { millis: 0 }`「只探测不挂起」同一形状的"只问"。
/// 收方据此备出装得下的缓冲，槽因此总能被排空。
pub fn pull_len(token: PieToken) -> EnvResult<(usize, TaskId)> {
    pull_from(token, core::ptr::null_mut(), 0)
}

/// pull 一条消息并取回**发送者**（`(长度, 发送者 TaskId)`）。
///
/// 发送者由内核在 `Push` 时盖章——身份不可伪造，不必再从报文里猜。
/// `max == 0` ⇒ 只报长度、不动槽（收方缓冲不参与）。
pub fn pull_from(token: PieToken, buf: *mut u8, max: usize) -> EnvResult<(usize, TaskId)> {
    let r = MailCall::Pull {
        token: token,
        buf: VirtAddr::new(buf as usize),
        max,
    }
    .call()?;
    match r {
        MailCallRet::Pull((n, from)) => Ok((n, from)),
        _ => unreachable!(),
    }
}

/// 等 hole 某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
/// 返回 `true` = 本次调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
///
/// 门铃（Nole）也走这一个：`dir` 必须给 [`HoleDir::Pull`]——铃只有"响了"一条方向，
/// 别的值内核答 `Denied`。裸函数层不为它另开一个名字：`Bell::wait` 就是这一句。
pub fn wait(token: PieToken, dir: HoleDir, millis: usize) -> EnvResult<bool> {
    let r = MailCall::Wait {
        token: token,
        dir,
        millis,
    }
    .call()?;
    match r {
        MailCallRet::Wait(ready) => Ok(ready),
        _ => unreachable!(),
    }
}

/// 响铃（门铃专用）：置"有待取之事"并唤醒听者。已响 → `Busy`。
pub fn ring(token: PieToken) -> EnvResult<()> {
    let r = MailCall::Ring { token: token }.call()?;
    match r {
        MailCallRet::Ring(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 应铃（门铃专用）：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
pub fn hush(token: PieToken) -> EnvResult<()> {
    let r = MailCall::Hush { token: token }.call()?;
    match r {
        MailCallRet::Hush(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 开闩：借映 Pole 页进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
///
/// **两件一起返**：起点与长度是同一段区间的两半，而长度只在内核手里（外来区按
/// 页界撑开，设备树 `reg` 声明的长度内核不知道）。
pub fn open(token: PieToken) -> EnvResult<(usize, usize)> {
    let r = PieCall::Open { token: token }.call()?;
    match r {
        PieCallRet::Open((va, size)) => Ok((va.get(), size)),
        _ => unreachable!(),
    }
}

pub fn shut(token: PieToken) -> EnvResult<()> {
    let r = PieCall::Shut { token: token }.call()?;
    match r {
        PieCallRet::Shut(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn seal(token: PieToken) -> EnvResult<()> {
    let r = PieCall::Seal { token: token }.call()?;
    match r {
        PieCallRet::Seal(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）。
///
/// 返回的是句柄而非裸数：它要经线形送到对方、再由对方 `from_token` 重建——
/// 全程一个 `PieToken`，中途不化成 `usize` 便不会与别的 id 混。
pub fn accord(src: PieToken, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
    let r = PieCall::Accord { src, dst, subset }.call()?;
    match r {
        PieCallRet::Accord(tk) => Ok(tk),
        _ => unreachable!(),
    }
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: PieToken, subset: env::Permission) -> EnvResult<()> {
    let r = PieCall::Narrow {
        token: token,
        subset,
    }
    .call()?;
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
pub fn collect(index: usize) -> EnvResult<(PieToken, env::Permission, TaskId)> {
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
    let r = PieCall::Release { token: token }.call()?;
    match r {
        PieCallRet::Release(()) => Ok(()),
        _ => unreachable!(),
    }
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

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
/// 镜像内核侧 `gate::AnyPie`（`enum { Hole, Pole }`，提供同一批跨种类方法）：
/// 同一条「权柄操作与资源种类无关」的知识在两侧各落一次，而不是散成四份。
pub trait AnyPie {
    /// 封印资源（**只有资源开辟者**可做）。
    ///
    /// 只置死并唤醒等待者，**不摘表项**——持有者仍须 [`release`](AnyPie::release)
    /// 收尾，否则表项泄漏。故 `release` 与 [`PolePie::shut`] 是**仅有的两处**不过存活闸
    /// 的操作（ABI 那一侧的两条注记同时写着这一条：`env::fid` 的 `Release` / `Shut`）。
    fn seal(&self) -> EnvResult<()>;

    /// 收窄本 pie 权限（就地改写，单调；`subset` ⊆ 当前权限）。
    ///
    /// Pole 多一条约束：`subset` 须含 FETCH（RISC-V PTE 无 R=0 的合法数据叶子），
    /// 且会同步把已映射段降权。Hole 无映射，故无此约束。
    fn narrow(&self, subset: env::Permission) -> EnvResult<()>;

    /// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）——
    /// 对方用 `from_token(at_dst)` 重建。
    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken>;

    /// 收回我授给 `dst` 的副本（含其全部后代，幂等）。
    ///
    /// `at_dst` = 该副本在**对端表里**的句柄（[`accord`](AnyPie::accord) 的返回值，
    /// 经线形送达）——**不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」。
    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()>;

    /// 放下我这一份（含其全部后代；Pole 同步撤映射）。资源本身不动——封印用
    /// [`seal`](AnyPie::seal)。不需要任何权限位。
    fn release(&self) -> EnvResult<()>;
}

/// Hole 门闩用户态句柄。
pub struct HolePie {
    token: PieToken,
}

impl HolePie {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的，见 [`unseal_hole`]）。
    pub fn unseal(mark: Mark) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_hole(mark)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 等某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
    /// 返回 `true` = 调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
    pub fn wait(&self, dir: HoleDir, millis: usize) -> EnvResult<bool> {
        wait(self.token, dir, millis)
    }

    /// 写消息（长度随消息，**无上限**）：槽满则睡到有空间（让出 CPU）。
    pub fn push(&self, msg: &[u8]) -> EnvResult<()> {
        loop {
            match push(self.token, msg.as_ptr(), msg.len()) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Push, usize::MAX)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 取消息：槽空则睡到有信（让出 CPU）。返实际收到字节数（≤ `buf.len()`）。
    ///
    /// **装不下（消息比 `buf` 长）返 `Denied`，且槽原样**——此时先问 [`HolePie::len`]
    /// 再备够缓冲，别丢。这里不替调用方把槽丢掉：丢一条消息是不可逆的。
    pub fn pull(&self, buf: &mut [u8]) -> EnvResult<usize> {
        loop {
            match pull(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(n) => return Ok(n),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Pull, usize::MAX)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 同 [`HolePie::pull`]，但一并取回**发送者**（内核盖章的 task id）。
    pub fn pull_from(&self, buf: &mut [u8]) -> EnvResult<(usize, TaskId)> {
        loop {
            match pull_from(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Pull, usize::MAX)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 只看一眼：槽里那条消息的**长度与发送者**，**一个字节都不取**（槽留原样）。
    ///
    /// 用途是 [`HolePie::pull`] 的前一步：缓冲不够大时先问长度、再备够。
    /// 槽空 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> EnvResult<(usize, TaskId)> {
        pull_len(self.token)
    }

    /// 有界 pull：槽空则最多等 `millis` 毫秒；仍无消息 → `Err(Busy)`（码 -3）。
    /// 用于「等对端回复」这类必须有上界的往返：无限等会把协议错误（回复被丢弃、
    /// 对端漏回）变成不可诊断的挂起。**超时后该 hole 不再"干净"**——迟到的回复
    /// 仍可能落进槽里，使下一次 pull 取到上一条；调用方应弃用该会话。
    ///
    /// 实现要点：`wait` 返 false **不等于**超时——它可能是「唤醒闩（pend）被消费」
    /// 或一次无关唤醒（见 `messenger::wake`：无等待者时置 pend，而成功裸 pull 不会
    /// 消费它，故 pend 可能是陈旧的）。所以这里按 **deadline 循环**：只有 `clock()`
    /// 真的走完 `millis` 才报 Busy，否则带着剩余时间重试。
    pub fn pull_timeout(&self, buf: &mut [u8], millis: usize) -> EnvResult<usize> {
        self.pull_timeout_from(buf, millis).map(|(n, _)| n)
    }

    /// 同 [`HolePie::pull_timeout`]，但一并取回**发送者**——「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间。
    pub fn pull_timeout_from(&self, buf: &mut [u8], millis: usize) -> EnvResult<(usize, TaskId)> {
        let deadline = now_ns()?.saturating_add((millis as u64).saturating_mul(1_000_000));
        loop {
            match pull_from(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    let now = now_ns()?;
                    if now >= deadline {
                        return pull_from(self.token, buf.as_mut_ptr(), buf.len());
                    }
                    let remain_ms = ((deadline - now) / 1_000_000).max(1) as usize;
                    let _ = self.wait(HoleDir::Pull, remain_ms)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

impl AnyPie for NolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(self.token, dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

impl AnyPie for PolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(self.token, dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

impl AnyPie for HolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(self.token, dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

/// Nole 门闩用户态句柄——**三种句柄里唯一没有自己那一套动词的那种**（它只有构造与
/// [`AnyPie`] 那一套；`HolePie` 多数据面、`PolePie` 多页视图）。
///
/// 它没有 `push`/`pull`（那是 Hole 的数据面）、没有 `open`/`shut`（那是 Pole 的
/// 页视图）。它能做的只有 [`AnyPie`] 那一套（`accord`/`narrow`/`revoke`/`release`
/// /`seal`）——因为它的全部内容就是"我持有这一枚"。
pub struct NolePie {
    token: PieToken,
}

impl NolePie {
    /// 解封一枚 Nole（无参数：没有大小、没有对齐）。
    pub fn unseal() -> EnvResult<Self> {
        Ok(Self {
            token: unseal_nole()?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// Pole 门闩用户态句柄。
pub struct PolePie {
    token: PieToken,
}

impl PolePie {
    /// 解封 Pole：一段页级安全内存（**大小页对齐**，清零）。
    ///
    /// 创建者的视图由内核顺手落好（`unseal` 内部 `auto-map`），但**那个 VA 不在这里回**
    /// ——要地址就再 `open` 一次（幂等，返同一个 VA）。
    pub fn unseal(size: usize) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_pole(size)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 开闩：借映进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
    ///
    /// 薄层只封这一次 envcall；"视图"这个用法在 [`crate::core::dock`]。
    pub fn open(&self) -> EnvResult<(usize, usize)> {
        open(self.token)
    }

    pub fn shut(&self) -> EnvResult<()> {
        shut(self.token)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

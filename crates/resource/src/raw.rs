//! 显式原始资源适配：孔 token 操作与能力表查询。
//!
//! 借入的 token 未经本模块验证，Hole 不拥有资源。Capability 管理本地创建的能力，Loan
//! 管理指定对端的派生授予；这些入口用于协议、硬件和测具边界。

use env::{Wait, HoleDir, MailFail, MailResult, Mark, PieResult, PieToken, TaskId, VirtAddr};

pub use crate::capability::{Loan, Capability};

/// Complete cleanup despite temporary contention, yielding between attempts.
pub fn release(token: PieToken) -> PieResult<()> {
    cleanup(|| env::pie::release(token))
}

pub fn revoke(peer: TaskId, token: PieToken) -> PieResult<()> {
    cleanup(|| env::pie::revoke(peer, token))
}

fn cleanup(mut run: impl FnMut() -> PieResult<()>) -> PieResult<()> {
    loop {
        match run() {
            Err(error) if error.source == env::PieFail::Busy => env::room::starve(),
            result => return result,
        }
    }
}

/// 单调时钟读数（纳秒）——deadline 用（机器无关，不依赖 timebase 频率）。
fn now_ns() -> u64 {
    env::chrono::clock()
}

fn deadline_of(within: Wait) -> u64 {
    match within {
        Wait::Forever => u64::MAX,
        Wait::AtMost(ms) => now_ns().saturating_add((ms as u64).saturating_mul(1_000_000)),
    }
}

/// 下一次进内核还要等多久。**至少 1 毫秒**：`AtMost(0)` 在那一侧不是"再看一眼"而是"挂 0 秒"。
fn remains(within: Wait, deadline: u64) -> Wait {
    if matches!(within, Wait::Forever) {
        return Wait::Forever;
    }
    match deadline.checked_sub(now_ns()) {
        None | Some(0) => Wait::AtMost(1),
        Some(d) => Wait::AtMost(((d / 1_000_000) as usize).max(1)),
    }
}

/// 只手递上去（`len ≥ 1`）。**内核那一格的一次尝试**，成不成看孔上有没有手。
fn put(token: PieToken, msg: &[u8]) -> MailResult<()> {
    env::mail::push(token, VirtAddr::new(msg.as_ptr() as usize), msg.len())
}

/// 只手取下来。**内核那一格的一次尝试**，手上有东西才成。
fn get(token: PieToken, buf: &mut [u8]) -> MailResult<(usize, TaskId)> {
    env::mail::pull(token, VirtAddr::new(buf.as_mut_ptr() as usize), buf.len())
}

/// Hole 门闩用户态句柄——**数据面那一枚**。
pub struct Hole {
    token: PieToken,
}

impl Hole {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的）。
    pub fn unseal(mark: Mark) -> PieResult<Self> {
        Ok(Self {
            token: env::pie::unseal_hole(mark)?,
        })
    }

    /// 由原始 token 重建句柄（用于协议边界接收 accord 来的 pie）。
    ///
    /// 不验证 token 是否存活、属于本任务或为 Hole；调用操作时由内核返回判决。
    pub fn from_raw(token: PieToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// **递出一条消息**：把 `msg` 那一手登记到本孔上（`len ≥ 1`；**内核当场复制进队列**）。
    ///
    /// `within` = **孔上站着别人的手时**允许等多久：
    ///
    ///   - `Wait::POLL` = 只试一次：孔上已有手 ⇒ `Busy`（"递完即走"那一半）；
    ///   - `AtMost(n)` / `Forever` = 等到**轮到我**（孔空）再递；预算内一直等不到 ⇒ 原样答 `Busy`。
    ///
    /// **`Ok` = 内核收下了这只手**，不是送达。送达（这只手被对侧取走）要 [`Hole::wait`]：
    /// `wait(HoleDir::Push, …)` 报的就是"孔空了"。**这一手不等自己那只手**——推的与取的是
    /// 同一个条件（见文件头），替调用方等会把"谁等谁"这条契约藏起来；要等就明写。
    pub fn push(&self, msg: &[u8], within: Wait) -> MailResult<()> {
        if matches!(within, Wait::AtMost(0)) {
            return put(self.token, msg);
        }
        let deadline = deadline_of(within);
        loop {
            match put(self.token, msg) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.is_busy() => {
                    // 到点 ⇒ 把最后一次尝试的结果原样交出去（`Busy` 也是答案）。
                    if now_ns() >= deadline
                        || !self.wait(HoleDir::Push, remains(within, deadline))?
                    {
                        return put(self.token, msg);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// **取走一只手**：把内核队列里的字节复制进 `buf`，返 `(实际长度, 发送者)`。
    ///
    /// `within` = **手上没东西时**允许等多久：`Wait::POLL` = 只试一次（手上没东西 ⇒ `Busy`）；
    /// `AtMost(n)` / `Forever` = 等到**有东西**（`Wait{HoleDir::Pull}` 就绪）。
    ///
    /// 装不下（`len > buf.len()`）返 `Denied`，**手原样留在孔上**——换够大的缓冲再来取，不丢消息。
    /// 要问长度用 [`Hole::peek`]。
    ///
    /// **发送者由内核在 `Push` 时盖章**——身份不可伪造，不必再从报文里猜；「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间，故这一手一并返回来。
    pub fn pull(&self, buf: &mut [u8], within: Wait) -> MailResult<(usize, TaskId)> {
        if matches!(within, Wait::AtMost(0)) {
            return get(self.token, buf);
        }
        let deadline = deadline_of(within);
        loop {
            match get(self.token, buf) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    if now_ns() >= deadline
                        || !self.wait(HoleDir::Pull, remains(within, deadline))?
                    {
                        return get(self.token, buf);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// **等某一侧就绪**：`true` = 就绪；`false` = 预算走完仍未就绪。
    ///
    /// `dir` = **我等在孔的哪一侧**：
    ///
    ///   - [`HoleDir::Pull`] = 取的人等"有东西"（有一只待取的手，或那一位是位）；
    ///   - [`HoleDir::Push`] = 推的人等"孔空"（**轮到我** ∧ **我那只手被取走**）。
    ///
    /// **这一手自己会循环**：内核返 `false` **不等于**预算到点——它可能是"唤醒闩（pend）被
    /// 消费"或一次无关唤醒（见 `messenger::wake`：无等待者时置 pend，而成功裸 pull 不会消费它，
    /// 故 pend 可能是陈旧的），也可能是 `Forever` 落成的"到不了的点"被板线程按 `BLIND_MS`
    /// （~100 ms）复探了一次。故这里是**按 deadline 的循环**：只有 `clock()` 真的走完 `within`
    /// 才报 `false`，否则带剩余时间重探。`Wait::Forever` 在这一层 = 一直等（deadline 到不了）。
    pub fn wait(&self, dir: HoleDir, within: Wait) -> MailResult<bool> {
        wait(self.token, dir, within)
    }

    /// **只看一眼**：孔上那只手的**长度、发送者、队里排着几只**，**一个字节都不取**（孔留原样）。
    ///
    /// 不动孔的状态（取用中的那只也照报），也不唤醒任何人。**不是取消息的前一步**：取走就是一次
    /// [`Hole::pull`]，够不够由 `buf.len()` 判。它的读者是"等之前先看一眼"那一格
    /// （`harness` 的 waiter：多个等待者挂在同一只组键上，要**非破坏性**地判"有货"）。
    /// 手上没东西 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> MailResult<(usize, TaskId, usize)> {
        env::mail::peek(self.token)
    }

    /// Atomically discard the queue head only when it exceeds max bytes.
    pub fn discard_oversized(&self, max: usize) -> MailResult<bool> {
        env::mail::discard(self.token, max)
    }

    /// **队里排着几只**（`Peek` 的第三格）：写者据此知道"我还排着几手"——孔上可以排着
    /// 至多 `QUEUE_CAP` 只手，故"我那只手被取走了没有"由这个数说。
    ///
    /// **空孔答 0**（不是错误）：一只都没排是写者要的那条事实。孔没了才答 `Dead`。
    pub fn depth(&self) -> MailResult<usize> {
        match env::mail::peek(self.token) {
            Ok((_, _, depth)) => Ok(depth),
            Err(e) if e.source == MailFail::Busy => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// **响这一位**：置"有待取之事"并唤醒听者。已响 → `Busy`。
    pub fn ring(&self) -> MailResult<()> {
        env::mail::ring(self.token)
    }

    /// **应这一位**：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
    ///
    /// `wait` 不清、要显式 `hush`：清必须与"取完"同一刻——醒来之后还要处理、处理完才轮得到
    /// "没有待取之事"。
    pub fn hush(&self) -> MailResult<()> {
        env::mail::hush(self.token)
    }
}

/// 表里的一枚（[`collect`] 收拢出来的那一格）——**三件事实一起**，故不必再问第二次。
///
/// 字段名就是判据：`owner` 是**资源**的来历（副本共享同一事实），与"谁授的"（`vestor`，
/// 转手即改写）**不是一回事**——那一格这一手不答，要问就走 [`reserve`]。
///
/// 两处哨兵与 `Reserve` 同一条口径：`TaskId(0)` = 这一格没有答案（原初自持 / 已封印 /
/// 不是孔），[`Mark::NONE`] = 记号那一格没有答案（记号只长在孔上）。
#[derive(Clone, Copy)]
pub struct Pie {
    /// 这一枚在本任务表里的号（`env::pie::unseal_hole` 那一族铸的）。
    pub token: PieToken,
    /// **这扇门谁开的**（副本共享同一事实）；`0` = 查不出（已封印 / 不是孔）。
    pub owner: TaskId,
    /// **这条路的名字**（`unseal_hole` 刻的那一格）；`NONE` = 这一枚不是孔。
    pub mark: Mark,
}

/// 收拢：本任务权限表第 `index` 份。越界 → 四格全哨兵（见 [`Pie`]），**不报错**。
///
/// **唯一的枚举手段**（[`reserve`] 是它的对偶：一个按位置问、一个按句柄问）。
/// 一次调用答四格，故"扫一遍这张表"**不必每一枚再问一次 `reserve`**——那一问是一次
/// envcall（~55 µs），表 16 枚 ⇒ 一趟扫描 6.5 ms（读数见
/// `programs/src/driver/rtc/adapt/desk.rs` 与 `444d1f3`）。
///
/// **不返 `Result`**：内核那一格恒写三件事实，没有失败支。
pub fn collect(index: usize) -> Pie {
    let (token, owner, mark) = env::pie::collect(index);
    Pie { token, owner, mark }
}

/// [`collect`] 那条枚举：**0 起、哨兵收尾、越界不报错**——这句话**只写这一处**。
///
/// **名字**：[`Collect`] 枚举的是整张权限表（孔 / 铃 / 页 / 组 / 别人给的副本都在里面），
/// 不只是孔。故叫 [`pies`]。
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
/// `programs/src/driver/router/adapt/desk.rs` 的 `drop_lane` 与 `programs/src/harness/guest/lodger/main.rs`）。
pub fn table_size() -> usize {
    pies().count()
}

/// 查询：我持有的这枚门闩——`(vestor, owner, 记号)`。
///
/// `vestor` = 这枚门闩谁授的（转手即改写）；`owner` = 这扇门谁开的（副本共享同一
/// 事实）；**记号** = **这条路的名字**（`unseal_hole` 刻的那一格，副本共享同一事实）。
/// 求「对端是谁」一律用 `owner`：root 转发过的门闩，`vestor` 会变成 root。
///
/// 三件事实由同一次调用**一起返回**（`a0` 打包 + `a1` 记号）：不再有"先问一格、再问另一格"
/// 那一趟。这一枚不是孔（记号只长在孔上）、或表里没有它 ⇒ `Denied`；**资源已封印 ⇒
/// `Dead`(-2)**——`owner` 那一格带存活闸，故"这一枚答不出"有两个码，别只接 `Denied`。
pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
    // 打包见 `env::abi::call` 的 `Reserve`：`a0` = owner 高半 | vestor 低半，`a1` = 记号。
    env::pie::reserve(token).map(|(pair, mark)| {
        (
            TaskId::new(pair & 0xffff_ffff),
            TaskId::new(pair >> 32),
            Mark::new(mark as u64),
        )
    })
}

/// **这一枚还在不在**（**与种类无关**：孔 / 页 / 铃 / 组都答得出）。
///
/// 两件一起判：在本任务表里 ＋ 没被封印 / 没交出去。与 [`reserve`] 的分工在它那一格：
/// 那一格答的是**孔的**来历与记号（对页与铃答 `Denied`），这一格只答存活这一件事实。
///
/// **不失败**：答不出就是 `false`（那一格不返负码）。
pub fn alive(token: PieToken) -> bool {
    env::pie::alive(token).unwrap_or(false)
}

/// Query the immediate transferor, original owner and mark of any live resource.
pub fn inspect(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
    env::pie::inspect(token).map(|(pair, mark)| {
        (
            TaskId::new(pair & 0xffff_ffff),
            TaskId::new(pair >> 32),
            Mark::new(mark as u64),
        )
    })
}

/// 等资源就绪；无关唤醒后继续使用剩余预算。
pub(crate) fn wait(token: PieToken, dir: HoleDir, within: Wait) -> MailResult<bool> {
    if env::mail::wait(token, dir, within)? {
        return Ok(true);
    }
    if matches!(within, Wait::AtMost(0)) {
        return Ok(false);
    }
    let deadline = deadline_of(within);
    loop {
        if now_ns() >= deadline {
            return Ok(false);
        }
        let step = remains(within, deadline);
        if env::mail::wait(token, dir, step)? {
            return Ok(true);
        }
    }
}

/// 借映共享页，返回映射起点与长度。
pub fn open(token: PieToken) -> PieResult<(usize, usize)> {
    env::pie::open(token).map(|(va, size)| (va.get(), size))
}

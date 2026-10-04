//! pie — **四枚门闩句柄**（Hole / Nole / Pole / Tole）＋ 它们共用的权柄动词。
//!
//! 句柄是**厚的一侧**：`PieToken` 只是内核那张表里的一个号，句柄把它与"怎么用"绑在一起
//! ——孔的三手（[`HolePie::push`] / [`HolePie::pull`] / [`HolePie::wait`]）带**期限循环**，
//! 页的视图（[`PolePie::open`] / [`PolePie::shut`]）把起止成对带回来，组把成员挂到一处。
//! 期限循环是政策，不是转发——故按 [`crate::core`] 的判据（薄/厚）住这里。
//!
//! **裸 envcall 不在这里**：`env::pie::seal(token)`、`env::mail::push(token, …)`、
//! `env::tole::attach(…)` 是 `crates/env` 生成的每格入口，句柄直接叫它们，不再经一层同名转发。
//!
//! # 三条口径
//!
//! - **`AnyPie`**：权柄操作（`Seal` / `Narrow` / `Accord` / `Revoke` / `Release`）与资源种类
//!   无关，故四份 `impl` **摆在一处**——散进四个文件就只剩四次重复。镜像内核侧
//!   `gate::AnyPie` 的四个变体提供同一批方法。
//! - **`Mate`**：能当组的一格成员的东西（孔 / 铃 / 页上那一位）——一个 trait 而不是收
//!   `PieToken`，让"能挂什么"在编译期说得清。
//! - **三个动词，等由参数说**：数据面只有 `push` / `pull` / `wait` 三手，`Wait::POLL`
//!   （= `AtMost(0)`）就是"只试一次"；`try_push` / `pull_timeout` 那一族都是预算的写法，
//!   留在调用点。
//!
//! # 推的人与取的人等的是**同一个内核条件**
//!
//! `ready(Push) = Idle`（孔空）与 `ready(Pull) = Hand | Rung`（有东西）是孔那一格状态轴的
//! 两个方向。推的人在 [`HoleDir::Push`] 上等"孔空"——它**同时**覆盖"轮到我"（孔上站着
//! 别人的手）与"我那只手被取走"；取的人在 [`HoleDir::Pull`] 上等"有东西"。故
//! [`HolePie::push`] 只等到**递得出去**；递出之后还要不要等它下线，是**第二件事**、
//! 由写端那一格显式说（`protocol::communication::hand` 的 `Sender::reclaim`）。
//!
//! # 无界等的白名单
//!
//! **"等"的尽头不许是"对面想起来"**。每一处 `Wait::Forever` 都要能指回三类之一：
//! 常驻事件等待（`Pile::await_` 那一族）、"字节必须活到被取走"（`push` 之后那一等）、
//! 等外部世界或必然会来的答复（控制台那一枚 `pull`、驱动那一族的 `recv`）。除这三类之外
//! **一律有界**（详见 `protocol::system::operator::client` 的 `RETRY_MIN_MS` 那条）。

use env::{
    Wait, HoleDir, MailFail, MailResult, Mark, Permission, PieResult, PieToken, Source, TaskId,
    ToleResult, VirtAddr,
};

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

// ── 四种资源的用户态句柄 ──────────────────────────────────────────────────

/// Hole 门闩用户态句柄——**数据面那一枚**。
pub struct HolePie {
    token: PieToken,
}

impl HolePie {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的）。
    pub fn unseal(mark: Mark) -> PieResult<Self> {
        Ok(Self {
            token: env::pie::unseal_hole(mark)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// **递出一条消息**：把 `msg` 那一手登记到本孔上（`len ≥ 1`；**不搬字节、不分配**）。
    ///
    /// `within` = **孔上站着别人的手时**允许等多久：
    ///
    ///   - `Wait::POLL` = 只试一次：孔上已有手 ⇒ `Busy`（"递完即走"那一半）；
    ///   - `AtMost(n)` / `Forever` = 等到**轮到我**（孔空）再递；预算内一直等不到 ⇒ 原样答 `Busy`。
    ///
    /// **`Ok` = 内核收下了这只手**，不是送达。送达（这只手被对侧取走）要 [`HolePie::wait`]：
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

    /// **取走一只手**：把发送方那段复制**一次**进 `buf`，返 `(实际长度, 发送者)`。
    ///
    /// `within` = **手上没东西时**允许等多久：`Wait::POLL` = 只试一次（手上没东西 ⇒ `Busy`）；
    /// `AtMost(n)` / `Forever` = 等到**有东西**（`Wait{HoleDir::Pull}` 就绪）。
    ///
    /// 装不下（`len > buf.len()`）返 `Denied`，**手原样留在孔上**——换够大的缓冲再来取，不丢消息。
    /// 要问长度用 [`HolePie::peek`]。
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
        if env::mail::wait(self.token, dir, within)? {
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
            if env::mail::wait(self.token, dir, step)? {
                return Ok(true);
            }
        }
    }

    /// **只看一眼**：孔上那只手的**长度、发送者、队里排着几只**，**一个字节都不取**（孔留原样）。
    ///
    /// 不动孔的状态（取用中的那只也照报），也不唤醒任何人。**不是取消息的前一步**：取走就是一次
    /// [`HolePie::pull`]，够不够由 `buf.len()` 判。它的读者是"等之前先看一眼"那一格
    /// （`harness` 的 waiter：多个等待者挂在同一只组键上，要**非破坏性**地判"有货"）。
    /// 手上没东西 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> MailResult<(usize, TaskId, usize)> {
        env::mail::peek(self.token)
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

/// Nole 门闩用户态句柄——**门铃**：四枚句柄里唯一没有自己那一套数据面的那种。
///
/// 它没有 `push`/`pull`（那是 Hole 的数据面）、没有 `open`/`shut`（那是 Pole 的页视图）。
/// 它能做的只有 [`AnyPie`] 那一套 ＋ 铃那一套（`wait` / `hush` / `ring`）——因为它的全部
/// 内容就是"一位 ＋ 我持有这一枚"。
pub struct NolePie {
    token: PieToken,
}

impl NolePie {
    /// 解封一枚 Nole（无参数：没有大小、没有对齐）。
    pub fn unseal() -> PieResult<Self> {
        Ok(Self {
            token: env::pie::unseal_nole()?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// **等铃响**。**没有方向参数**：铃只有一条方向（有事 / 没事），签名少一个参数就把这件事
    /// 说完了。返回 `true` = 当场就绪（未挂起）；`false` = 预算走完仍未就绪。**不清**那一位
    /// ——见 [`NolePie::hush`]。
    pub fn wait(&self, within: Wait) -> MailResult<bool> {
        HolePie::from_token(self.token).wait(HoleDir::Pull, within)
    }

    /// 响铃：置"有待取之事"并唤醒听者。已响 → `Busy`。
    pub fn ring(&self) -> MailResult<()> {
        env::mail::ring(self.token)
    }

    /// 应铃：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
    pub fn hush(&self) -> MailResult<()> {
        env::mail::hush(self.token)
    }
}

/// Pole 门闩用户态句柄——**页视图那一枚**（也带着**页上那一位"有事"**）。
pub struct PolePie {
    token: PieToken,
}

impl PolePie {
    /// 解封 Pole：一段页级安全内存（**大小页对齐**，清零）。
    ///
    /// 创建者的视图由内核顺手落好（`unseal` 内部 `auto-map`），但**那个 VA 不在这里回**
    /// ——要地址就再 `open` 一次（幂等，返同一个 VA）。
    pub fn unseal(size: usize) -> PieResult<Self> {
        Ok(Self {
            token: env::pie::unseal_pole(size, true)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 开闩：借映进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
    pub fn open(&self) -> PieResult<(usize, usize)> {
        open(self.token)
    }

    pub fn shut(&self) -> PieResult<()> {
        env::pie::shut(self.token)
    }

    /// **响一下页上那一位**：置"有待取之事"并唤醒听者（已响 ⇒ `Busy`，不是错）。
    ///
    /// 页上为什么有"有事"：架（`protocol::communication::rack`）把铃并进页 ⇒ 一枚页就是一具
    /// 完整的架。它**不是中断响的**：`hush` 不碰本 hart 的闸门（与孔上那一位同一条）。
    pub fn ring(&self) -> MailResult<()> {
        env::mail::ring(self.token)
    }

    /// **应一下**：清掉"有待取之事"。已经清着 ⇒ `Busy`（调用方当"正好"）。
    pub fn hush(&self) -> MailResult<()> {
        env::mail::hush(self.token)
    }

    /// **等那一位亮**。与 [`NolePie::wait`] 同一形：只有"有事"一条方向（没有 `dir` 参数），
    /// `true` = 当场就绪（未挂起）。**不清**那一位——清要显式 [`PolePie::hush`]。
    pub fn wait(&self, within: Wait) -> MailResult<bool> {
        HolePie::from_token(self.token).wait(HoleDir::Pull, within)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// 组的用户态句柄（与 [`HolePie`] 同款：只持一枚号，资源实体在内核）。
///
/// 方法集 = 这一个对象上能做的三件事（`unseal` 是**构造**，做不成 `&self` 方法）。
pub struct TolePie {
    token: PieToken,
}

impl TolePie {
    /// 造一个空组。
    ///
    /// `shared` = 这枚组允不许多个使用者（**造的时候定、之后不可变**，
    /// `ToleCall::Unseal`）：`false` = 独占组（授出即移交、复制不出来），`true` = 共享组
    /// （可 `accord` 复制给多个任务；组键的唤醒是提示型——放行全链）。
    pub fn unseal(shared: bool) -> ToleResult<Self> {
        Ok(Self {
            token: env::tole::unseal(shared)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的组）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 把一枚成员的一个方向挂进来（同成员幂等）。
    pub fn attach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        env::tole::attach(self.token, mate.token(), dir)
    }

    /// 摘掉一格；没挂过即无事。
    pub fn detach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        env::tole::detach(self.token, mate.token(), dir)
    }

    /// 把一个**状态来源**登记进组（同 `(source, target)` 幂等）。
    ///
    /// 登记成功即留一次待复核提示；`target` 的合法性（组需本地独占、能力变化只能看自己、
    /// 任务收尾的授权同 `Join`）由内核核。
    pub fn subscribe(&self, source: Source, target: TaskId) -> ToleResult<()> {
        env::tole::subscribe(self.token, source, target)
    }

    /// 按**已安装的订阅描述**取消；同描述重复取消无事。
    pub fn unsubscribe(&self, source: Source, target: TaskId) -> ToleResult<()> {
        env::tole::unsubscribe(self.token, source, target)
    }

    /// 等到组里任意一格有事：`(哪一枚, 哪个方向)`；`millis` 上限族，同全树。
    ///
    /// `PieToken::NONE` = 没等到（或挂起过——见 `ToleCall::Await`）。
    /// **组上装了状态订阅时，它还有第二义**："有来源报过事，去复核"——它从不等于
    /// "肯定没有变化"。
    /// **这一格不循环**：组的返回是**提示**（"快照变了"），"等到没有"是调用点的循环
    /// （见 `programs/src/harness/bench/group/waiter/main.rs`）。
    pub fn await_(&self, millis: Wait) -> ToleResult<(PieToken, HoleDir)> {
        env::tole::await_(self.token, millis)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// 能当**一格成员**的东西：孔、铃、以及**页上那一位**。
///
/// 与内核侧 `mail::tole::Mate` 是同一条边界：**架把"有事"给了页** ⇒ 页也能进组
/// （`Pole(PoleId)`，只有 `Pull` 一条方向）；组也不进组（没有位，判据会变成沿图的递归）。
/// 用一个 trait 而不是收 `PieToken`，是为了让"能挂什么"在编译期就说得清。
pub trait Mate {
    /// 本成员在**我这张表**里的号。
    fn token(&self) -> PieToken;
}

impl Mate for HolePie {
    fn token(&self) -> PieToken {
        HolePie::token(self)
    }
}

impl Mate for NolePie {
    fn token(&self) -> PieToken {
        NolePie::token(self)
    }
}

impl Mate for PolePie {
    fn token(&self) -> PieToken {
        PolePie::token(self)
    }
}

// ── 记号类适配：内核那几格的口径 → 调用方口径 ──────────────────────────────

/// 开闩：借映 Pole 页进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
///
/// **两件一起返**：起点与长度是同一段区间的两半，而长度只在内核手里（外来区按
/// 页界撑开，设备树 `reg` 声明的长度内核不知道）。
pub fn open(token: PieToken) -> PieResult<(usize, usize)> {
    env::pie::open(token).map(|(va, size)| (va.get(), size))
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

// ── 类型化句柄：**权柄面**（构造 + 种类无关那几手）──

/// 权柄句柄 —— Hole / Pole / Nole / Tole 的**权柄操作同构**，故只写一遍。
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
pub trait AnyPie {
    /// 封印资源（**只有资源开辟者**可做）。
    ///
    /// 只置死并唤醒等待者，**不摘表项**——持有者仍须 [`release`](AnyPie::release)
    /// 收尾，否则表项泄漏。故 `release` 与 `PolePie::shut` 是**仅有的两处**不过存活闸
    /// 的操作（ABI 那一侧的两条注记同时写着这一条：`env::abi::call` 的 `Release` / `Shut`）。
    fn seal(&self) -> PieResult<()>;

    /// 收窄本 pie 权限（就地改写，单调；`subset` ⊆ 当前权限）。
    ///
    /// Pole 多一条约束：`subset` 须含 FETCH（RISC-V PTE 无 R=0 的合法数据叶子），
    /// 且会同步把已映射段降权。Hole 无映射，故无此约束。
    fn narrow(&self, subset: Permission) -> PieResult<()>;

    /// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）——
    /// 对方用 `from_token(at_dst)` 重建。`mark` = 子枚的记号（`NONE` = 照源枚）。
    fn accord(&self, dst: TaskId, subset: Permission, mark: Mark) -> PieResult<PieToken>;

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
// 四份都只是把 `self.token()` 递给 `env::pie` 那几个入口——**没有一份多一行**：那正是
// trait 头注那句"权柄操作与资源种类无关"的读法（`revoke` 的那一格收的是**对端**的句柄，
// 故四份都不看 `self.token()`，那一处不算例外）。

impl AnyPie for HolePie {
    fn seal(&self) -> PieResult<()> {
        env::pie::seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        env::pie::narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission, mark: Mark) -> PieResult<PieToken> {
        env::pie::accord(self.token(), dst, subset, mark)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        env::pie::revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        env::pie::release(self.token())
    }
}

impl AnyPie for NolePie {
    fn seal(&self) -> PieResult<()> {
        env::pie::seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        env::pie::narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission, mark: Mark) -> PieResult<PieToken> {
        env::pie::accord(self.token(), dst, subset, mark)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        env::pie::revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        env::pie::release(self.token())
    }
}

impl AnyPie for PolePie {
    fn seal(&self) -> PieResult<()> {
        env::pie::seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        env::pie::narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission, mark: Mark) -> PieResult<PieToken> {
        env::pie::accord(self.token(), dst, subset, mark)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        env::pie::revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        env::pie::release(self.token())
    }
}

/// 组（Tole）的那一份——与前三份逐字同构。它成立的前提在内核侧：`gate::AnyPie` 的
/// `Tole` 变体在 `seal` / `narrow` / `accord` / `revoke` / `release` 五条路上都有人接
/// （`envcall/pie.rs`、`gate/narrow.rs`、`gate/accord.rs`、`gate/cull.rs`）——
/// 少一条，这一份就是假接口。
///
/// 共享组（`ToleCall::Unseal { shared: true }`）本来就要经 `accord` 才到得了多个任务
/// （"共享组若不可复制，'多个使用者'是空话"），故这一份不是补上去的摆设。
impl AnyPie for TolePie {
    fn seal(&self) -> PieResult<()> {
        env::pie::seal(self.token())
    }

    fn narrow(&self, subset: Permission) -> PieResult<()> {
        env::pie::narrow(self.token(), subset)
    }

    fn accord(&self, dst: TaskId, subset: Permission, mark: Mark) -> PieResult<PieToken> {
        env::pie::accord(self.token(), dst, subset, mark)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> PieResult<()> {
        env::pie::revoke(dst, at_dst)
    }

    fn release(&self) -> PieResult<()> {
        env::pie::release(self.token())
    }
}

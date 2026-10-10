//! 显式原始资源适配：孔 token 操作与能力表查询。
//!
//! 借入的 token 未经本模块验证，Hole 不拥有资源。Capability 管理本地创建的能力，Loan
//! 管理指定对端的派生授予；这些入口用于协议、硬件和测具边界。

use env::{
    MailCondition, MailFail, MailResult, Mark, Oversize, PieInfo, PieKind, PieResult, PieToken,
    PullOutcome, TaskId, VirtAddr, Wait,
};

pub use crate::capability::{Capability, Loan};

/// Complete cleanup despite temporary contention, yielding between attempts.
pub fn release(token: PieToken) -> PieResult<()> {
    cleanup(|| env::pie::release(token, env::ReleaseMode::Revoke))
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

/// Hole 门闩用户态句柄——**数据面那一枚**。
pub struct Hole {
    token: PieToken,
}

impl Hole {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的）。
    pub fn unseal(mark: Mark) -> PieResult<Self> {
        Ok(Self {
            token: env::pie::unseal(env::UnsealArgs::hole(mark))?,
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

    /// Copy one message into the kernel queue, retrying until space or the deadline.
    /// Success means queued. Wait(Empty) observes a drained queue, not completed processing.
    /// Push readiness does not promise that a particular message fits the byte budget.
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
                        || !self.wait(MailCondition::Push, remains(within, deadline))?
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
    /// `AtMost(n)` / `Forever` = 等到**有东西**（`Wait{MailCondition::Pull}` 就绪）。
    ///
    /// 装不下（`len > buf.len()`）返 `Denied`，**手原样留在孔上**——换够大的缓冲再来取，不丢消息。
    /// 要问长度用 [`Hole::peek`]。
    ///
    /// **发送者由内核在 `Push` 时盖章**——身份不可伪造，不必再从报文里猜；「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间，故这一手一并返回来。
    pub fn pull(&self, buf: &mut [u8], within: Wait) -> MailResult<(usize, TaskId)> {
        match self.pull_with(buf, within, Oversize::Keep)? {
            PullOutcome::Received { len, sender } => Ok((len, sender)),
            PullOutcome::Discarded { .. } => unreachable!("Keep cannot discard"),
        }
    }

    /// Wait for Pull (readable), Push (space available), or Empty (queue drained).
    /// Unrelated wakeups recheck the condition using the original deadline.
    pub fn wait(&self, dir: MailCondition, within: Wait) -> MailResult<bool> {
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

    /// Receive one message with an explicit policy for an oversized head.
    pub fn pull_with(
        &self,
        buf: &mut [u8],
        within: Wait,
        oversize: Oversize,
    ) -> MailResult<PullOutcome> {
        let deadline = if within == Wait::POLL {
            0
        } else {
            deadline_of(within)
        };
        loop {
            let attempt = env::mail::pull(
                self.token,
                VirtAddr::new(buf.as_mut_ptr() as usize),
                buf.len(),
                oversize,
            );
            match attempt {
                Err(ref error)
                    if error.source.is_busy() && within != Wait::POLL && now_ns() < deadline =>
                {
                    if !self.wait(MailCondition::Pull, remains(within, deadline))? {
                        return attempt;
                    }
                }
                result => return result,
            }
        }
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
        env::mail::ring(self.token, env::Bit::FIRST)
    }

    /// **应这一位**：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
    ///
    /// `wait` 不清、要显式 `hush`：清必须与"取完"同一刻——醒来之后还要处理、处理完才轮得到
    /// "没有待取之事"。
    pub fn hush(&self) -> MailResult<()> {
        env::mail::hush(self.token, env::Bit::FIRST)
    }
}

/// A capability table entry, including kind, permissions and liveness.
pub type Pie = PieInfo;

/// Increasing token cursor; removals do not shift the next position.
/// Batches observe current state rather than freezing the whole table.
pub struct Pies {
    after: PieToken,
    words: [usize; PieInfo::WORDS * 8],
    at: usize,
    count: usize,
    done: bool,
}
impl Iterator for Pies {
    type Item = Pie;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.at == self.count {
            self.count = env::pie::collect(
                self.after,
                VirtAddr::new(self.words.as_mut_ptr() as usize),
                8,
            )
            .expect("capability table read failed");
            self.at = 0;
            if self.count == 0 {
                self.done = true;
                return None;
            }
        }
        let start = self.at * PieInfo::WORDS;
        let info = PieInfo::from_words(
            self.words[start..start + PieInfo::WORDS]
                .try_into()
                .unwrap(),
        )
        .expect("invalid capability table entry");
        self.at += 1;
        self.after = info.token;
        Some(info)
    }
}
pub fn pies() -> Pies {
    Pies {
        after: PieToken::NONE,
        words: [0; PieInfo::WORDS * 8],
        at: 0,
        count: 0,
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

/// Query a held reference, including a sealed resource's owner and mark.
pub fn inspect(token: PieToken) -> PieResult<PieInfo> {
    let mut words = [0; PieInfo::WORDS];
    env::pie::inspect(token, VirtAddr::new(words.as_mut_ptr() as usize))?;
    Ok(PieInfo::from_words(words).expect("invalid PieInfo"))
}

/// Hole provenance convenience wrapper over Inspect.
pub fn reserve(token: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
    let info = inspect(token)?;
    if !info.alive {
        return Err(env::make_fail(env::PieFail::Dead));
    }
    if info.kind != PieKind::Hole {
        return Err(env::make_fail(env::PieFail::Denied));
    }
    Ok((info.vestor, info.owner, info.mark))
}

pub fn alive(token: PieToken) -> bool {
    inspect(token).is_ok_and(|info| info.alive)
}

/// 等资源就绪；无关唤醒后继续使用剩余预算。
pub(crate) fn wait(token: PieToken, dir: MailCondition, within: Wait) -> MailResult<bool> {
    let deadline = if within == Wait::POLL {
        0
    } else {
        deadline_of(within)
    };
    if env::mail::wait(token, dir, within)? {
        return Ok(true);
    }
    if matches!(within, Wait::AtMost(0)) {
        return Ok(false);
    }
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

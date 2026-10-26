//! Hole 门闩用户态句柄——**数据面那一枚**。

use env::{
    HoleCondition, HoleResult, Mark, Oversize, PieToken, PullOutcome, TaskId, VirtAddr, Wait,
};

use super::{deadline_of, now_ns, put, remains, wait};

pub struct Hole {
    token: PieToken,
}

impl Hole {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的）。
    pub fn unseal(mark: Mark) -> HoleResult<Self> {
        Ok(Self {
            token: env::hole::unseal(mark, env::HoleLimits::default())?,
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
    pub fn push(&self, msg: &[u8], within: Wait) -> HoleResult<()> {
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
                        || !self.wait(HoleCondition::Push, remains(within, deadline))?
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
    /// `AtMost(n)` / `Forever` = 等到**有东西**（`Wait{HoleCondition::Pull}` 就绪）。
    ///
    /// 装不下（`len > buf.len()`）返 `Denied`，**手原样留在孔上**——换够大的缓冲再来取，不丢消息。
    /// 要问长度用 [`Hole::peek`]。
    ///
    /// **发送者由内核在 `Push` 时盖章**——身份不可伪造，不必再从报文里猜；「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间，故这一手一并返回来。
    pub fn pull(&self, buf: &mut [u8], within: Wait) -> HoleResult<(usize, TaskId)> {
        match self.pull_with(buf, within, Oversize::Keep)? {
            PullOutcome::Received { len, sender } => Ok((len, sender)),
            PullOutcome::Discarded { .. } => unreachable!("Keep cannot discard"),
        }
    }

    /// Wait for Pull (readable), Push (space available), or Empty (queue drained).
    /// Unrelated wakeups recheck the condition using the original deadline.
    pub fn wait(&self, dir: HoleCondition, within: Wait) -> HoleResult<bool> {
        wait(self.token, dir, within)
    }

    /// **只看一眼**：孔上那只手的**长度、发送者、队里排着几只**，**一个字节都不取**（孔留原样）。
    ///
    /// 不动孔的状态（取用中的那只也照报），也不唤醒任何人。**不是取消息的前一步**：取走就是一次
    /// [`Hole::pull`]，够不够由 `buf.len()` 判。它的读者是"等之前先看一眼"那一格
    /// （`harness` 的 waiter：多个等待者挂在同一只组键上，要**非破坏性**地判"有货"）。
    /// 手上没东西 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> HoleResult<(usize, TaskId, usize)> {
        env::hole::peek(self.token)
    }

    /// Receive one message with an explicit policy for an oversized head.
    pub fn pull_with(
        &self,
        buf: &mut [u8],
        within: Wait,
        oversize: Oversize,
    ) -> HoleResult<PullOutcome> {
        let deadline = if within == Wait::POLL {
            0
        } else {
            deadline_of(within)
        };
        loop {
            let attempt = env::hole::pull(
                self.token,
                VirtAddr::new(buf.as_mut_ptr() as usize),
                buf.len(),
                oversize,
            );
            match attempt {
                Err(ref error)
                    if error.source.is_busy() && within != Wait::POLL && now_ns() < deadline =>
                {
                    if !self.wait(HoleCondition::Pull, remains(within, deadline))? {
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
    pub fn depth(&self) -> HoleResult<usize> {
        match env::hole::peek(self.token) {
            Ok((_, _, depth)) => Ok(depth),
            Err(e) if e.source == env::HoleFail::Busy => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// **响这一位**：置"有待取之事"并唤醒听者。已响 → `Busy`。
    pub fn ring(&self) -> HoleResult<()> {
        env::hole::ring(self.token)
    }

    /// **应这一位**：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
    ///
    /// `wait` 不清、要显式 `hush`：清必须与"取完"同一刻——醒来之后还要处理、处理完才轮得到
    /// "没有待取之事"。
    pub fn hush(&self) -> HoleResult<()> {
        env::hole::hush(self.token)
    }
}

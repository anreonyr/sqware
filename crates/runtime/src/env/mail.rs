//! Mail 域 —— **通信面**：门闩（pie）那一族"怎么用"的那一半。
//!
//! # 两条轴，两个文件
//!
//! `env::fid` 文件头把 `PieCall`（class 7）与 `MailCall`（class 5）立成两条正交的轴
//! （权柄 / 数据）。本层按轴分文件：
//!
//!   - **通信面（本文件）**：class 5 的 `Push` / `Pull` / `Peek` / `Wait` / `Hush` / `Ring`，加
//!     class 9（`ToleCall`：一枚"组"的造 / 挂 / 摘 / 等）。组是**多路等待**——成员是孔的
//!     一个方向或一枚铃，故它接着本文件那一族的等待语义（分界见 `env::fid`：
//!     "本类不搬载荷"）；
//!   - **权柄面**（[`pie`](super::pie)）：class 7 的转发 + [`AnyPie`] 与它的四份实现。
//!
//! # 四种资源的用户态句柄都住本文件
//!
//! Hole（数据面）、Nole（门铃）、Pole（页视图）、Tole（组）——它们的**权柄面**
//! （`Seal` / `Narrow` / `Accord` / `Revoke` / `Release`）由 [`AnyPie`] 提供，四份实现
//! 集中在 `pie.rs`。口径一句话：**句柄一处（本文件），权柄动词一处（`pie.rs`）**。
//!
//! Pole 不属通信面，它住这里是因为"句柄一处"这一条：它的数据面是页视图
//! （`open` / `shut`），而页视图与权柄一样，与"往哪推、从哪收"无关。
//!
//! # 名字照旧（本文件为什么有一大段 `pub use`）
//!
//! 权柄轴拆去 `pie.rs` 之后，`runtime::env::mail::X` 这一形（`protocol` / `programs` /
//! `harness` 里十几处调用点）**一行没改**：下面把 `pie.rs` 的每一项按名字转出去。
//! 这是本仓搬家的既有先例（`Access`/`Policy`、`Announce`/`Grant`/`Died` 都是这么转的）。
//!
//! # 三个动词，等由参数说
//!
//! 数据面只有三手：[`HolePie::push`]（递出一条报）/ [`HolePie::pull`]（取走一条报）/
//! [`HolePie::wait`]（等某一侧就绪）。加 [`HolePie::peek`]（只看一眼，不动孔）与那一格"位"
//! 的 `ring` / `hush`，就是 class 5 那一张表的全部。
//!
//! **默认不睡**：`Wait::POLL`（= `AtMost(0)`）就是"只试一次"，要等就把预算写进 `within`。
//! 三条动词各带**一次** `Wait`，而"到点没有"那个循环全层只有一处（[`HolePie::wait`]）。
//! 从前那一族 `try_push` / `try_pull` / `pull_timeout` / `pull_timeout_from` / `pull_from` /
//! `pull_len` / `hand_out` / `hand_out_turn` / `hand_back`，连同占了大半本文件的**裸转发函数
//! 层**（class 5 八条 ＋ class 9 四条），**一并退了场**：它们每一个都是上面这三手的一种预算写法。
//!
//! # 推的人与取的人等的是**同一个内核条件**
//!
//! `ready(Push) = Idle`（孔空）与 `ready(Pull) = Hand | Rung`（有东西）是孔那一格状态轴的
//! 两个方向，故：
//!
//!   - 推的人在 [`HoleDir::Push`] 上等"孔空"——它**同时**覆盖两件事：**轮到我**（孔上站着
//!     **别人**的手）与**我那只手被取走**（孔上站着**我**的手）；
//!   - 取的人在 [`HoleDir::Pull`] 上等"有东西"。
//!
//! 两者是同一个条件的两面。故 [`HolePie::push`] 里那一等只管**递出之前**（轮到我），
//! **递出之后**还要不要等它下线，是**第二件事**、由写端那一格显式说
//! （`protocol::communication::sender` 的 `Sender::reclaim`）——载体不替调用方藏
//! "谁等谁"这条契约。
//!
//! **照实记（"递出之后的兜底期限"退场了，附判决）**：这里原先有一格 `HANDOFF_MS = 1000`：
//! 递出手之后等对面来取，等满就把**那条报撤掉**（旧动词 `Withdraw`）、答 `Busy`。它的本意是
//! "不让发送方永久挂着"，实测却把**"对面慢"折成了"这条报作废"**，而作废不可逆：
//!
//! - 直接跑那一趟（boot.nu 那一路）量到：驱动起手向设备账认领时，`face_of` 的 `"hub"` 步与
//!   hub `bond` 那一手失败（`tid=11 reason=0x9 note: hub`／`tid=10 reason=0x5 note: bond`／
//!   `tid=12 reason=0xc note: bond`），接着 rtc 的装配失败（`tid=2 note: system: assemble`）⇒
//!   编排域退出 ⇒ 级联，整机只剩 6 条 `exit`，而**同一配方在 `27d4267` 是 15/15 `reason=0x0`**。
//! - 判决实验（只摘掉"等对方来取"那一段，其余不动）：**3 跑全干净**（无 `system: assemble`、
//!   无驱动失败、无 panic），基线 3 跑全带病。⇒ 病灶就是这一段，不是树、不是线程。
//!
//! 今天的口径：**"撤手"这条路不存在**——`Withdraw` 这一格连同它的唯一用家一起退了场（用户
//! 裁定"不留"）。"不等了"由**所有权**解决：递出去的字节住在写端那一格里
//! （`protocol::communication::sender`），它的 `Drop` 负责"等手下线"；而内核那一侧
//! `Hand.space` 是**弱引用**（`work/mail/hole.rs`），发送方真走了，后来那次 `Pull` 答 `Gone`
//! ——内核从没钉住发送方那段内存，故"悬着"这件事本来就有两个正当收场，不需要第三格"撤手"。
//!
//! **照实记（它替掉的观察，别丢）**：那一格当年的用处之一是"把永久挂降成**可诊断**的一笔"。
//! 那个价值由内核收场那一行接走：`Push` 方向等超过 1 秒会记账，收场时与
//! `timer:`/`doom:`/`sched:` 一起打出来（见 `kernel/src/work/room/conductor.rs` 的 `hole:`）。
//!
//! # 无界等的白名单（**一条可查的判据**）
//!
//! 口径：**"等"的尽头不许是"对面想起来"**。每一处 `Wait::Forever` 都要能指回下面三类之一；
//! 指不回去的，就是一处"谁也说不清什么时候会动"的等待。判据可 grep：
//! `grep -rn 'Wait::Forever' programs/src crates/*/src`。
//!
//!   1. **常驻事件等待**：服务 / 驱动那个循环在等"下一件事"（`Pile::await_` 那一族，以及
//!      看一条死亡道的 `service::watch`）。等的不是某一位对端的手 ⇒ **这一格就该无界**
//!      （有界就成了轮询）。
//!   2. **"字节必须活到被取走"**：`push` 之后那一等（`door.wait(HoleDir::Push, …)`）——它护的是
//!      "调用方那段内存这一步不许复用"。给它期限就等于把"对面慢"折成"这条报作废"
//!      （上面 `HANDOFF_MS` 那条照实记：**实测过，会级联**）。
//!   3. **等"外部世界"或"必然会来的答复"**：控制台那一枚 `pull`（没人按键就是没人按键）、
//!      驱动那一族的 `recv`（本域那条协议必然有答）。这一类**不设期限是对的**——"等不来"正是
//!      该有的症状；**如实记**：它今天**没有读数**（内核只给 `Push` 方向记账，"等信是常态"），
//!      要它就得另立一行。
//!
//! 除这三类之外**一律有界**：客侧敲门与收答（`AtMost`）、"译不出的路"那一重试（退避 ＋ 真时限，
//! 见 `protocol::service::operator::client` 的 `RETRY_MIN_MS`）、装配期问一格（`AtMost(MS)`）。
//!
//! **照实记（这一条是量与查两条腿里"查"的那一条）**：debug 档 `product` 景里量到过"同一枚孔被
//! 连问 500／1000 次而整机不前进"（`operator: woke n=500…2000 tok=501 known=true read=true`）
//! ——那既是**一场风暴**（客侧重试按毫秒扣账、两轮 1 ms），也是**一处说不清的等**。风暴那一条已收；
//! 这一张白名单是**下一刀（"一客一格 ＋ 待答账"）的判据**：那时三类之外会多出一类必须处理的对象
//! ——**服务侧答话的收口**（今天它就是 `Sender::reclaim` 的无界等）。

use env::Wait;
use env::{HoleDir, MailResult, Mark, PieResult, PieToken, TaskId, ToleResult, VirtAddr};

/// 单调时钟读数（纳秒）——deadline 用（机器无关，不依赖 timebase 频率）。内核那一格没有
/// 失败支，故跟着 [`clock`](crate::env::chrono::clock) 一起不返 `Result`。
fn now_ns() -> u64 {
    crate::env::chrono::clock()
}

/// 预算的终点（纳秒）。**`Forever` 落成"一个到不了的点"**——照实记：内核的续等点被
/// `min(最近活到点, chrono::timer::BLIND_MS)` 收着（内核那一侧见 `chrono/timer.rs`），
/// 故"到不了的点" = **每 ~100 ms 被叫醒一次、自己复探**；那一层复探就是这条等待今天的护栏。
/// 落成真永久（不武装定时器）要先动内核那一格——**不在这一刀里**。
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

// ── 权柄轴（class 7）搬去 `pie.rs` 之后的名字照旧 ──────────────────────────
//
// 整面转出（**不挑**）：转发是"路径不变"的保证，一旦按"今天谁在用"挑，下一个调用点就得
// 先认出这层壳才知道自己该写 `pie::`——那正是这一层想免掉的认知成本。
pub use super::pie::{
    AnyPie, Pie, Pies, accord, collect, narrow, open, pies, release, reserve, revoke, seal, shut,
    table_size, unseal_hole, unseal_nole, unseal_pole,
};

// ── 四种资源的用户态句柄 ──────────────────────────────────────────────────

/// Hole 门闩用户态句柄——**数据面那一枚**。
///
/// 它的权柄面不在这里：`Seal` / `Narrow` / `Accord` / `Revoke` / `Release` 由 `pie.rs`
/// 的 [`AnyPie`] impl 提供（见本文件头注）。这里只有构造与数据面。
pub struct HolePie {
    token: PieToken,
}

impl HolePie {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的，见 [`unseal_hole`]）。
    pub fn unseal(mark: Mark) -> PieResult<Self> {
        Ok(Self {
            token: super::pie::unseal_hole(mark)?,
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
    ///
    /// **照实记（"等轮到自己"这半格是从"整台机器起不来"量回来的）**：四个门面客侧（名册 /
    /// 盟册 / 控制面 / 设备账）为了绕开"两个服务互等对方取走"那条死锁（名册 ⇄ 盟册，量到过），
    /// 一度把这一手写成"只递出"（旧名 `hand_out`，即今天的 `within = POLL`）＋ 收完再等。
    /// **形状是对的，但连 `Busy` 一起丢了**——孔上正站着另一位客人的手时，这一趟**当场**折成
    /// `Fail::Denied`，而"等轮到自己"原来住在旧 `push` 的循环里。
    ///
    /// 症状（一路量下来的）：孔是单槽 ⇒ 装配期装配者替客人 `derive` 一条身份（名册那一面）与
    /// 树的门禁（**同一个面**，每问一次受门禁的请求都要问它）撞车 ⇒ `Error::Step("derive")`
    /// ⇒ 装配当场收场（`system: assemble`）⇒ 下游一片"服务缺席"（驱动死在 `hub` 步、客人没上树、
    /// 问话永远没人读）。**单看那一趟只是 `Denied`，看不出是撞车。**
    ///
    /// **陷阱（`dir` 那一格）**：`Busy` 只说明"孔上有手"，不说是谁的——递出**之前**等的是别人的
    /// 手，递出**之后**等的是自己的手，而两件事在同一个 `Wait{HoleDir::Push}` 上。故"等到哪一步
    /// 为止"必须由调用点自己说清：这一手只等到"递得出去"，"等到被取走"要另写一次
    /// [`HolePie::wait`]（写端那一格 `Sender::reclaim` 就是那一笔）。
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
                    if now_ns() >= deadline || !self.wait(HoleDir::Push, remains(within, deadline))? {
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
    /// 发送方那段已经没了（或它那个空间已回收）返 `Gone`。要问长度用 [`HolePie::peek`]。
    ///
    /// **发送者由内核在 `Push` 时盖章**——身份不可伪造，不必再从报文里猜；「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间，故这一手一并返回来。
    ///
    /// **`Forever` 落成"一个到不了的点"**（`deadline_of` 那一条照实记）：内核按 `BLIND_MS`
    /// （~100 ms）复探一次，故这一等的护栏是**这一层的循环**，不是一次 envcall。
    pub fn pull(&self, buf: &mut [u8], within: Wait) -> MailResult<(usize, TaskId)> {
        if matches!(within, Wait::AtMost(0)) {
            return get(self.token, buf);
        }
        let deadline = deadline_of(within);
        loop {
            match get(self.token, buf) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    if now_ns() >= deadline || !self.wait(HoleDir::Pull, remains(within, deadline))? {
                        return get(self.token, buf);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// **等某一侧就绪**：`true` = 就绪；`false` = 预算走完仍未就绪。
    ///
    /// `dir` = **我等在孔的哪一侧**（照直觉）：
    ///
    ///   - [`HoleDir::Pull`] = 取的人等"有东西"（有一只待取的手，或那一位是位）；
    ///   - [`HoleDir::Push`] = 推的人等"孔空"（**轮到我** ∧ **我那只手被取走**）。
    ///
    /// **这一手自己会循环**：内核返 `false` **不等于**预算到点——它可能是"唤醒闩（pend）被
    /// 消费"或一次无关唤醒（见 `messenger::wake`：无等待者时置 pend，而成功裸 pull 不会消费它，
    /// 故 pend 可能是陈旧的），也可能是 `Forever` 落成的"到不了的点"被板线程按 `BLIND_MS`
    /// （~100 ms）复探了一次。故这里是**按 deadline 的循环**：只有 `clock()` 真的走完 `within`
    /// 才报 `false`，否则带剩余时间重探。`Wait::Forever` 在这一层 = 一直等（deadline 到不了）。
    ///
    /// **它是最省事的那一手**：写端"不到手不罢休"就是 `wait(HoleDir::Push, Wait::Forever)` 一句
    /// ——没有期限那一档（旧 `hand_back` 的 `u64::MAX` 循环已经收进这里）。
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

    /// **只看一眼**：孔上那只手的**长度与发送者**，**一个字节都不取**（孔留原样）。
    ///
    /// 不动孔的状态（取用中的那只也照报），也不唤醒任何人。**不是取消息的前一步**：取走就是一次
    /// [`HolePie::pull`]，够不够由 `buf.len()` 判。它的读者是"等之前先看一眼"那一格
    /// （`harness` 的 waiter：多个等待者挂在同一只组键上，要**非破坏性**地判"有货"）。
    /// 手上没东西 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> MailResult<(usize, TaskId)> {
        env::mail::peek(self.token)
    }

    /// **响这一位**：置"有待取之事"并唤醒听者。已响 → `Busy`。
    ///
    /// 位（`Rung`）是孔那一格状态轴的第三格，与"有手"互斥；它是**状态**（"这一条有事"），
    /// 故置位即返、从不睡——通知只能是通知，不能是移交（照实记：旧孔时代路由者投递、客户说
    /// 排空两边都等 ⇒ 谁也回不去取自己那一格，机器当场不动；本刀把这两条通知都换成位，那条
    /// 互锁于是**结构上不可能**）。
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

/// Nole 门闩用户态句柄——**门铃**：四枚句柄里唯一没有自己那一套数据面的那种（它只有构造与
/// [`AnyPie`] 那一套；`HolePie` 多数据面、`PolePie` 多页视图、`TolePie` 多挂摘等）。
///
/// 它没有 `push`/`pull`（那是 Hole 的数据面）、没有 `open`/`shut`（那是 Pole 的页视图）。
/// 它能做的只有 [`AnyPie`] 那一套（`accord`/`narrow`/`revoke`/`release`/`seal`）＋
/// 铃那一套（`wait` / `hush` / `ring`）——因为它的全部内容就是"一位 ＋ 我持有这一枚"。
pub struct NolePie {
    token: PieToken,
}

impl NolePie {
    /// 解封一枚 Nole（无参数：没有大小、没有对齐）。
    pub fn unseal() -> PieResult<Self> {
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

    /// **等铃响**。**没有方向参数**：铃只有一条方向（有事 / 没事），签名少一个参数就把这件事
    /// 说完了，不必写注释解释"为什么只有 Pull"。返回 `true` = 当场就绪（未挂起）；
    /// `false` = 预算走完仍未就绪。**不清**那一位——见 [`NolePie::hush`]。
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

/// Pole 门闩用户态句柄——**页视图那一枚**。
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
            token: unseal_pole(size)?,
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
        shut(self.token)
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
    /// `shared` = 这枚组允不许多个使用者（**造的时候定、之后不可变**，见 `env::fid` 的
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

    /// 等到组里任意一格有事：`(哪一枚, 哪个方向)`；`millis` 上限族，同全树。
    ///
    /// `PieToken::NONE` = 没等到（或挂起过——见 `env::fid` 的 `ToleCall::Await`）。
    /// **这一格不循环**：组的返回是**提示**（"快照变了"），"等到没有"是调用点的循环
    /// （见 `harness/src/waiter.rs`：契约就是"别把一次返回当终局"）。
    pub fn await_(&self, millis: Wait) -> ToleResult<(PieToken, HoleDir)> {
        env::tole::await_(self.token, millis)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// 能当**一格成员**的东西：孔与铃。
///
/// 与内核侧 `mail::tole::Mate` 是同一条边界：页不进组（没有"有事"这回事），组也不
/// 进组（没有位，判据会变成沿图的递归）。用一个 trait 而不是收 `PieToken`，是为了
/// 让"能挂什么"在编译期就说得清。
///
/// `token` 不在 [`AnyPie`] 里（那一位是"表示层转换，与 ABI 无关"）；
/// 本 trait 的存在理由正是要那个号，故它自带一支。
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

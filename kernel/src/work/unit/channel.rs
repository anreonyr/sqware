// Channel — 一对一消息专线（内核侧引擎，v1 同空间 task↔task，mpsc 式交接）
//
// 双通道服务形态：`ChannelBuilder::spawn` 一次建 **req / resp 两条** SPSC 无锁
// ring（各自独立 `Channel` 对象），四端点一次吐出（mpsc「创建即双端」）：
//   Spawned { client: ClientEnd { req_tx, resp_rx },   // 客户端半对（留己用）
//             server: ServerEnd { req_rx, resp_tx } }  // 服务端半对（move 进对端）
//
// 消息 = 槽字（标量/地址/句柄通吃）；ring 上每条消息 = [len][data..]（长度槽 +
// 数据槽），`len` 即 ABI 实现细节（A6：槽片是内核 ABI 实现细节，用户信息类型
// 各自实现，不设编解码 trait）。pull/timeout 返回实际数据槽数（A7.5）。
//
// SPSC 无锁纪律（写入者纪律）：
//   head 只被推端写（tail 只被拉端写）——两指针单调递增 usize（不取模），
//   物理槽位 = (head + i) % slot_len；跨环尾拆两段写/读。
//   写序：数据（Relaxed）→ head.store(Release)；读序：head.load(Acquire) →
//   数据；读者 tail.store(Release)、写者 tail.load(Acquire) 同理。Release/
//   Acquire 对闭合可见性，无需 CAS（SPSC 无竞争）。
//
// 四态状态机（A1）：Live → Hang（推端消逝：Tx drop）→ Gone（Hang∧空且拉端
//   钉连）/ Dead（crush 或 Rx drop）。crush 单通道级、消费式（A7.4）。
//   阻塞唤醒 = tick 粒度 park 重试（复用 scheduler park；精确 waiters 属 v2）：
//   push 满 / pull 空 → ktask_park（本引擎 v1 面向内核任务，闭包体内自切换）。
//
// 共享帧形态（v1 内核侧）：`Channel` 即双端共享的内核对象（Arc<Channel> 两端
//   直访同一地址空间，AMO 零 trap——「不同 VA 映射同一 PA」的用户双端映射属
//   ucall 话题，后续接入）。先做内核侧引擎 + 内核任务验收。

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::runtime::chrono::clock::Instant;

/// 通道状态（四态，A1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChannelState {
    /// 正常（推端可推、拉端可拉）。
    Live = 0,
    /// 推端消逝（Tx drop 时 CAS Live→Hang，温和断开；余信仍可取）。
    Hang = 1,
    /// Hang ∧ 余信取空，拉端钉连（pull 于 Hang∧空 → CAS(Hang→Gone)）。
    Gone = 2,
    /// 终止：crush（任一端）或拉端消逝（Rx drop）→ 后续操作报 Dead。
    Dead = 3,
}

/// 通道错误（五变体，A7.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    /// 用法错：消息槽数超 slot_len、out 短于消息（槽超限归此）。
    Invalid,
    /// 断开感知（拉端）：推端消逝（Hang）且余信取空（→ Gone）。
    Gone,
    /// try_push 于环满。
    Full,
    /// timeout 到期未取到。
    Timeout,
    /// crush / Rx drop 之后任何操作。
    Dead,
}

/// 通道（内核对象，Arc 共享）——SPSC 无锁 ring，双端共享同一对象（v1 内核侧）。
pub struct Channel {
    /// 四态状态机（双端都可能改：crush / drop 钩子；CAS + fetch 语义）。
    state: AtomicU8,
    /// 写指针（推端独占写；拉端 Acquire 读）。
    head: AtomicUsize,
    /// 读指针（拉端独占写；推端 Acquire 读）。
    tail: AtomicUsize,
    /// ring 容量（槽字个数；不变）。
    slot_len: usize,
    /// 槽区（长度 = slot_len；物理索引 = 逻辑 % slot_len）。
    slots: Box<[AtomicUsize]>,
}

impl Channel {
    /// 双端共享同一 Channel；构造即引用计数归双方（strong = 2 + 构造侧临时）。
    fn new(slot_len: usize) -> Self {
        let n = slot_len.max(1);
        Self {
            state: AtomicU8::new(ChannelState::Live as u8),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            slot_len: n,
            slots: (0..n).map(|_| AtomicUsize::new(0)).collect(),
        }
    }

    fn state(&self) -> ChannelState {
        match self.state.load(Ordering::Acquire) {
            0 => ChannelState::Live,
            1 => ChannelState::Hang,
            2 => ChannelState::Gone,
            _ => ChannelState::Dead,
        }
    }

    /// 消息槽数 = 1（长度槽）+ slots.len()。槽超限 → Invalid（A7.6）。
    fn message_slots(slots: &[usize]) -> Option<usize> {
        let n = slots.len().checked_add(1)?;
        Some(n)
    }

    /// 当前占用（head - tail，wrapping 防御理论溢出：ring 容量远小于 usize::MAX）。
    fn used(head: usize, tail: usize) -> usize {
        head.wrapping_sub(tail)
    }

    /// 推进写指针前的空闲槽数：slot_len - used。
    fn free(&self, head: usize, tail: usize) -> usize {
        self.slot_len - Self::used(head, tail).min(self.slot_len)
    }

    /// 写 n 槽到逻辑位置 base（逐槽 mod 寻址，跨环尾自然分段）。
    fn store_slots(&self, base: usize, words: &[usize]) {
        for (i, w) in words.iter().enumerate() {
            self.slots[(base + i) % self.slot_len].store(*w, Ordering::Relaxed);
        }
    }

    /// 读 n 槽自逻辑位置 base → out（逐槽 mod 寻址）。
    fn load_slots(&self, base: usize, n: usize, out: &mut [usize]) {
        for i in 0..n {
            out[i] = self.slots[(base + i) % self.slot_len].load(Ordering::Relaxed);
        }
    }

    /// 非阻塞投递：满 → Full。消息 = [len][data..]。
    fn try_push(&self, slots: &[usize]) -> Result<(), ChannelError> {
        // 状态检查：push 侧只可能 Live 或 Dead（Hang/Gone 属推端消逝后，推端
        // 已不存在；Rx drop 置 Dead → 推端感知）。
        match self.state() {
            ChannelState::Live => {}
            ChannelState::Dead => return Err(ChannelError::Dead),
            _ => return Err(ChannelError::Gone), // 防御：构造不可达（Hang/Gone 时无人可 push）
        }
        let msg = Self::message_slots(slots).ok_or(ChannelError::Invalid)?;
        if msg > self.slot_len {
            return Err(ChannelError::Invalid); // 槽超限（A7.6）
        }
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if self.free(head, tail) < msg {
            return Err(ChannelError::Full);
        }
        // 零分配投递：长度槽 + 数据槽分两段直写（引擎无临时缓冲）。
        let len_slot = [slots.len()];
        self.store_slots(head, &len_slot);
        if !slots.is_empty() {
            self.store_slots(head.wrapping_add(1), slots);
        }
        // Release：数据先于 head 可见（读者 Acquire 读 head 后必见数据）。
        self.head.store(head.wrapping_add(msg), Ordering::Release);
        Ok(())
    }

    /// 非阻塞取：空 → Err(Invalid 由 out 短触发；空 → 返回 Err(ChannelError::Timeout)？
    /// 不——空是阻塞路径，try 面由调用方循环 + park。这里返回 None 语义由
    /// `try_pull_once`（内部枚举）表达；公开面只剩阻塞 pull / timeout。
    fn try_pull_once(&self, out: &mut [usize]) -> PullOutcome {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        if Self::used(head, tail) == 0 {
            return PullOutcome::Empty;
        }
        // 读长度槽（在 ring 上，head 指针前；长度必 ≤ slot_len - 1）
        let len = self.slots[tail % self.slot_len].load(Ordering::Acquire);
        if len > out.len() {
            // out 短于消息：不消费（head/tail 未动），调用方可换缓冲重试。
            return PullOutcome::TooSmall(len);
        }
        let need = 1 + len; // 长度槽 + 数据
        debug_assert!(Self::used(head, tail) >= need, "SPSC 不变量：占用 ≥ 消息");
        // 跳过长度槽，读 len 个数据槽
        self.load_slots(tail.wrapping_add(1), len, out);
        // Release：tail 前进释放槽（写者 Acquire 读 tail 后可见新空间）。
        self.tail.store(tail.wrapping_add(need), Ordering::Release);
        PullOutcome::Ok(len)
    }

    /// 阻塞取：空 → park 重试（tick 粒度）；Hang∧空 → CAS(Hang→Gone) → Gone。
    fn pull(&self, out: &mut [usize]) -> Result<usize, ChannelError> {
        loop {
            match self.state() {
                ChannelState::Dead => return Err(ChannelError::Dead),
                ChannelState::Gone => return Err(ChannelError::Gone),
                // Live：正常取；Hang：余信仍可取（取空后钉连 Gone）。
                ChannelState::Live | ChannelState::Hang => {}
            }
            match self.try_pull_once(out) {
                PullOutcome::Ok(n) => return Ok(n),
                PullOutcome::TooSmall(_) => return Err(ChannelError::Invalid), // out 短 → Invalid
                PullOutcome::Empty => {
                    // 空：Hang → 钉连 Gone（断开感知）；Live → park 重试
                    if self.state() == ChannelState::Hang {
                        let _ = self.state.compare_exchange(
                            ChannelState::Hang as u8,
                            ChannelState::Gone as u8,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        return Err(ChannelError::Gone);
                    }
                    crate::runtime::switcher::selfpark::ktask_park(
                        core::time::Duration::from_millis(100),
                    );
                }
            }
        }
    }

/// timeout：限时取；到 deadline 未至 → Timeout。
    fn timeout(&self, out: &mut [usize], deadline: Instant) -> Result<usize, ChannelError> {
        loop {
            match self.state() {
                ChannelState::Dead => return Err(ChannelError::Dead),
                ChannelState::Gone => return Err(ChannelError::Gone),
                // Live：正常取；Hang：余信仍可取（取空后钉连 Gone）。
                ChannelState::Live | ChannelState::Hang => {}
            }
            match self.try_pull_once(out) {
                PullOutcome::Ok(n) => return Ok(n),
                PullOutcome::TooSmall(_) => return Err(ChannelError::Invalid), // out 短 → Invalid
                PullOutcome::Empty => {
                    if self.state() == ChannelState::Hang {
                        let _ = self.state.compare_exchange(
                            ChannelState::Hang as u8,
                            ChannelState::Gone as u8,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        return Err(ChannelError::Gone);
                    }
                    // Live 且空：deadline 已过 → Timeout；未过 → park 至 deadline。
                    let left = ms_until(deadline);
                    if left.is_zero() {
                        return Err(ChannelError::Timeout);
                    }
                    crate::runtime::switcher::selfpark::ktask_park(left);
                }
            }
        }
    }
}

/// pull 单次尝试的结果（内部枚举：空/缓冲不足/成功；终态 Gone/Dead 由
/// 状态的显式检查覆盖，不在此枚举）。
enum PullOutcome {
    Ok(usize),
    Empty,
    TooSmall(usize),
}

/// crush：置 Dead（CAS Live/Hang → Dead；任一端调用；单通道级）+ 唤醒阻塞者
/// （tick 粒度：park 者自醒后见 Dead）。消费式（self 拿走本端 Arc）。
fn crush(channel: &Channel) {
    // fetch 而非 CAS：双端并发 crush / drop 竞态下置 Dead 是单向终点，达标即可。
    channel.state.store(ChannelState::Dead as u8, Ordering::Release);
}

fn clock_now() -> Instant {
    crate::runtime::chrono::clock::now()
}

fn ms_until(deadline: Instant) -> core::time::Duration {
    deadline
        .checked_duration_since(clock_now())
        .unwrap_or(core::time::Duration::ZERO)
}

// ── 方向类型义务（A5.3）：push 只收 Tx、pull 只收 Rx ──

/// 推端端点。Drop = 推端消逝 → CAS(Live→Hang)（温和断开，A1 任务退出挂钩）。
pub struct Tx {
    channel: Arc<Channel>,
}

impl Drop for Tx {
    fn drop(&mut self) {
        // 推端离开：Live → Hang；已是 Hang/Gone/Dead 则不回退（compare_exchange
        // 只在 Live 时成功）。gp 语义：Hang 让拉端可感知断开。
        let _ = self.channel.state.compare_exchange(
            ChannelState::Live as u8,
            ChannelState::Hang as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl Tx {
    /// 投递槽字消息；满 → park（tick 粒度重试）；Dead → Dead；槽超限 → Invalid。
    pub fn push(&self, slots: &[usize]) -> Result<(), ChannelError> {
        loop {
            match self.channel.try_push(slots) {
                Ok(()) => return Ok(()),
                Err(ChannelError::Full) => {
                    // 满 → park 重试（tick 粒度；唤醒后再查状态/空间）
                    crate::runtime::switcher::selfpark::ktask_park(
                        core::time::Duration::from_millis(100),
                    );
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 非阻塞投递；满 → Full。
    pub fn try_push(&self, slots: &[usize]) -> Result<(), ChannelError> {
        self.channel.try_push(slots)
    }

    /// 消费式终止（A7.4）：置 Dead + 唤醒阻塞者；本端 Arc 随消费释放。
    pub fn crush(self) {
        crush(&self.channel);
    }
}

/// 拉端端点。Drop = 拉端消逝 → CAS(Live→Dead)（对称补齐：推端感知 Dead）。
pub struct Rx {
    channel: Arc<Channel>,
}

impl Drop for Rx {
    fn drop(&mut self) {
        // 拉端离开：Live → Dead（推端 push 报 Dead，免于挂死，A 补丁裁决）。
        let _ = self.channel.state.compare_exchange(
            ChannelState::Live as u8,
            ChannelState::Dead as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

impl Rx {
    /// 取消息到 out（零分配）；空 → park；返回实际数据槽数。
    pub fn pull(&self, out: &mut [usize]) -> Result<usize, ChannelError> {
        self.channel.pull(out)
    }

    /// 非阻塞取（与 [`Tx::try_push`] 对称）；空 → `ItEmpty` 内部语义经
    /// `ChannelError::Timeout`? 否——空是阻塞路径；此处空 → `ChannelError::Gone`
    /// 语义不清。给公开面：空 → `Err(ChannelError::Timeout)` 用词不当。改用
    /// `Option`：空 → `Ok(None)`；终态 → `Err`。
    pub fn try_pull(&self, out: &mut [usize]) -> Result<Option<usize>, ChannelError> {
        match self.channel.state() {
            crate::work::unit::channel::ChannelState::Dead => Err(ChannelError::Dead),
            crate::work::unit::channel::ChannelState::Gone => Err(ChannelError::Gone),
            _ => match self.channel.try_pull_once(out) {
                PullOutcome::Ok(n) => Ok(Some(n)),
                PullOutcome::Empty => Ok(None),
                PullOutcome::TooSmall(_) => Err(ChannelError::Invalid),
            },
        }
    }

    /// 限时取；超时 → Timeout；返回实际数据槽数。
    pub fn timeout(&self, out: &mut [usize], deadline: Instant) -> Result<usize, ChannelError> {
        self.channel.timeout(out, deadline)
    }

    /// 消费式终止（同 Tx）。
    pub fn crush(self) {
        crush(&self.channel);
    }
}

// ── 创建面 ──

/// 通道构建器：`.slot_len(n).spawn()`（无 to/Peer——mpsc 式无指名，A7.7 配套）。
pub struct ChannelBuilder {
    /// ring 容量（槽字）；缺省 8（A7.3：2 的幂，索引免取模）。
    slot_len: usize,
}

impl ChannelBuilder {
    pub fn new() -> Self {
        Self { slot_len: 8 }
    }

    /// ring 容量（槽字；2 的幂最佳，任意 > 0 亦可——逐槽 mod 寻址不受限）。
    pub fn slot_len(mut self, n: usize) -> Self {
        self.slot_len = n.max(1);
        self
    }

    /// 建 req/resp 两条通道，四端点一次吐出（mpsc 式创建即双端）。
    pub fn spawn(self) -> Result<Spawned, ChannelError> {
        let req = Arc::new(Channel::new(self.slot_len));
        let resp = Arc::new(Channel::new(self.slot_len));
        Ok(Spawned {
            client: ClientEnd {
                req_tx: Tx {
                    channel: req.clone(),
                },
                resp_rx: Rx { channel: resp.clone() },
            },
            server: ServerEnd {
                req_rx: Rx { channel: req },
                resp_tx: Tx { channel: resp },
            },
        })
    }
}

impl Default for ChannelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 双通道产出：客户端半对留己用，服务端半对 move 进对端任务闭包。
pub struct Spawned {
    pub client: ClientEnd,
    pub server: ServerEnd,
}

/// 客户端半对：发请求（req_tx）+ 收响应（resp_rx）。
pub struct ClientEnd {
    pub req_tx: Tx,
    pub resp_rx: Rx,
}

/// 服务端半对：收请求（req_rx）+ 发响应（resp_tx）。
pub struct ServerEnd {
    pub req_rx: Rx,
    pub resp_tx: Tx,
}
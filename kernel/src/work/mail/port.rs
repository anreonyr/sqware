// port — 内核邮路（mail 三通道之一）：消息拷贝经内核。
//
// 定长小消息（[`MSG_LEN`]）、单槽：槽空 push 即存（拷贝一次入内核缓冲）；槽满
// push → Busy（调用方阻塞，用户侧条件循环 + env::wait 等槽空）；pull 同理（槽空
// → Busy 等投递）。条件变更方（push 成功 / pull 成功）env::wake 唤醒对端——内核
// 只管槽与拷贝，阻塞语义全部落到调度域 wait/wake。
//
// 生命周期（Gate 5 + port 补完，与 ring/dock 同型）：mail 接入点由 Task.mail
// 直接持有，无中间簿记。任务死 → MailHolds::drop → Vec<Port> 析构 → 末位 drop
// 经 `PortMeta::Drop` 置 Dead。
//
// 跨任务共享：open 方持首份入本方 mail；对端经 `port::join(handle)` 查全局
// `Weak<PortMeta>` → upgrade → 新造 Port（共享同一 `Arc<PortMeta>`）入本方 mail。
// 与 ring/dock 完全同型——单 Weak 注册表 + Arc<PortMeta> 一统状态与键。
//
// 键面：port 条件键 = `NEXT_KEY.fetch_add(1)`（全局唯一；与 DOCK/RING 不同——不
// 带 KEY_TAG，wait/wake 经 envcall 合成空间身份 WaitKey::compose）。

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::work::room::scheduler::core::current;

/// port 消息定长（内核邮路的槽字大小；栈拷贝，无动态分配）。
pub const MSG_LEN: usize = 64;

/// 端口条件键分配器（全局唯一；两端同持，经 envcall 合成空间身份）。
static NEXT_KEY: AtomicUsize = AtomicUsize::new(0);
/// 端口句柄分配器（envcall 面标识：句柄 → 注册表条目）。
static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// port 错误（D1 负码：#1 = Dead，>1 = 预留）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortError {
    /// 通道已 shut（对端断开感知）。
    Dead,
    /// 条件不满足（槽满 / 槽空）——调用方应阻塞（wait）后重试，非错误终结态。
    Busy,
}

/// 端口状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortState {
    Live,
    Dead,
}

struct PortInner {
    state: PortState,
    /// 单槽：Some = 消息在途（push 已存、pull 未取）。
    slot: Option<[u8; MSG_LEN]>,
}

/// 端口元数据（Arc 共享，与 ring::RingMeta / dock::DockMeta 同位）：
/// 状态锁 + 条件键同住一 Arc——join 升级 Weak 后元数据自洽，**无需第二张表**。
pub struct PortMeta {
    inner: SpinLock<PortInner>,
    /// 条件键（用户侧 wait/wake 用；两端共享）。
    key: usize,
}

impl Drop for PortMeta {
    fn drop(&mut self) {
        // 末位 drop（Arc 归零 = 全部 task.mail 已卸下）：置 Dead，状态语义收敛。
        // 同 ring::RingMeta / dock::DockMeta 末位 drop 守门——本函数只在 Arc
        // 计数真正归零时跑一次（Arc 守门由 Port 卸下顺序保证，详 ring/dock 注释）。
        self.inner.lock().state = PortState::Dead;
    }
}

/// 端口句柄：每任务一份（task.mail 持有）；多任务共享 `Arc<PortMeta>`。
pub struct Port {
    /// 句柄（全局唯一；push/pull 凭此查本方 mail）。
    handle: usize,
    /// 元数据（inner + key；多 Port 同 Arc）。
    meta: Arc<PortMeta>,
}

impl Port {
    /// 建新端口：分配 PortMeta + 注册到全局 Weak 表 + 分配句柄/键。
    /// 返回 `(Port, 条件键)`——调用方负责把 Port push 进当前 task.mail。
    fn create() -> (Port, usize) {
        let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        let meta = Arc::new(PortMeta {
            inner: SpinLock::new_level(
                Level::L3,
                PortInner {
                    state: PortState::Live,
                    slot: None,
                },
            ),
            key,
        });
        ports().lock().insert(handle, Arc::downgrade(&meta));
        (Port { handle, meta }, key)
    }
}

// ── 注册表（仅 join lookup：handle → Weak<PortMeta>）──

/// port 全局注册表（Level::L3；不主动清，Weak 升级失败后 lookup 返 None）。
/// 同 ring::rings / dock::docks 模式——单 Weak 表 + Arc<Meta>。
fn ports() -> &'static SpinLock<HashMap<usize, Weak<PortMeta>>> {
    static REG: OnceLock<SpinLock<HashMap<usize, Weak<PortMeta>>>> = OnceLock::new();
    REG.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

// ── 核心 API（envcall 入口）──

/// 建 port（envcall PortOpen 入口）：a0 = 句柄，a1 = 条件键。
/// 接入点 move 到当前 task.mail。
pub fn open() -> (usize, usize) {
    let (p, key) = Port::create();
    let handle = p.handle;
    let task = current()
        .running_task()
        .expect("port::open: envcall context");
    task.mail.lock().ports.push(p);
    (handle, key)
}

/// 加入已有 port（envcall PortJoin 入口）：a0 = 句柄 → a0 = 条件键。
/// 从全局 Weak 表查 PortMeta Arc（升级失败 = 原句柄已死 → Err）；升级成功 → 新
/// 造 Port 入本方 mail。两侧各持一份，Arc 保活至最末 drop（PortMeta::drop 置 Dead）。
pub fn join(handle: usize) -> Result<usize, PortError> {
    let meta = ports()
        .lock()
        .get(&handle)
        .and_then(Weak::upgrade)
        .ok_or(PortError::Dead)?;
    let key = meta.key;
    let task = current()
        .running_task()
        .expect("port::join: envcall context");
    task.mail.lock().ports.push(Port { handle, meta });
    Ok(key)
}

/// 终止 port（envcall PortShut 入口）：a0 = 句柄。从本方 mail 移除。
/// 当前 task 未持 `id` → false。Port 卸下 → Arc 递减；末位 drop 触发
/// PortMeta::drop 置 Dead。
pub fn shut(handle: usize) -> bool {
    let Some(task) = current().running_task() else {
        return false;
    };
    let mut mail = task.mail.lock();
    let Some(pos) = mail.ports.iter().position(|p| p.handle == handle) else {
        return false;
    };
    mail.ports.remove(pos);
    true
}

/// 投递（envcall PortPush 入口）：消息拷贝已由调用方（mail::copy_in）完成。
/// 本方 mail 未持此句柄 → Dead（对端已失效 / 跨任务须先 join）。
pub fn try_push(handle: usize, msg: &[u8; MSG_LEN]) -> Result<(), PortError> {
    let Some(task) = current().running_task() else {
        return Err(PortError::Dead);
    };
    // 借出 PortMeta Arc 后立即放 mail 锁——再锁 inner（L3）。mail 锁护的仅是
    // ports Vec，inner 自带锁，顺序获取不嵌套 → 3→3 同层不冲突。
    let meta = {
        let mut mail = task.mail.lock();
        let Some(p) = mail.ports.iter().find(|p| p.handle == handle) else {
            return Err(PortError::Dead);
        };
        Arc::clone(&p.meta)
    };
    let mut st = meta.inner.lock();
    if st.state == PortState::Dead {
        return Err(PortError::Dead);
    }
    if st.slot.is_some() {
        return Err(PortError::Busy);
    }
    st.slot = Some(*msg);
    Ok(())
}

/// 收取（envcall PortPull 入口）：拷贝出内核由调用方（mail::copy_out）完成。
pub fn try_pull(handle: usize) -> Result<[u8; MSG_LEN], PortError> {
    let Some(task) = current().running_task() else {
        return Err(PortError::Dead);
    };
    let meta = {
        let mut mail = task.mail.lock();
        let Some(p) = mail.ports.iter().find(|p| p.handle == handle) else {
            return Err(PortError::Dead);
        };
        Arc::clone(&p.meta)
    };
    let mut st = meta.inner.lock();
    if st.state == PortState::Dead {
        return Err(PortError::Dead);
    }
    match st.slot.take() {
        Some(msg) => Ok(msg),
        None => Err(PortError::Busy),
    }
}

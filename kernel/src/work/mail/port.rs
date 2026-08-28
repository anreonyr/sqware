// port — 内核邮路（mail 双通道之一）：消息拷贝经内核。
//
// 定长小消息（[`MSG_LEN`]）、单槽：槽空 push 即存（拷贝一次入内核缓冲）；槽满
// push → Busy（调用方阻塞，用户侧条件循环 + env::wait 等槽空）；pull 同理（槽空
// → Busy 等投递）。条件变更方（push 成功 / pull 成功）env::wake 唤醒对端——内核
// 只管槽与拷贝，阻塞语义全部落到调度域 wait/wake。
//
// 生命周期：open 建端口（句柄 = Arc 共享，两端同持）；shut → Dead：对端
// push/pull 返回 [`PortError::Dead`]（断开感知）。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};

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

/// 端口句柄：Arc 共享（两端 clone 同持）；shut 置 Dead。
#[derive(Clone)]
pub struct Port {
    inner: Arc<SpinLock<PortInner>>,
    /// 端口条件键（用户侧 wait/wake 用；open 时随句柄一并返回，两端同值）。
    key: usize,
}

impl Port {
    /// 建新端口：返回 (句柄, 条件键)。同空间两端持有同一句柄（clone），
    /// wait/wake 用同一 key——首版不变量（见 mod 文档）。
    pub fn open() -> (Port, usize) {
        let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        let p = Port {
            inner: Arc::new(SpinLock::new_level(
                Level::L3,
                PortInner {
                    state: PortState::Live,
                    slot: None,
                },
            )),
            key,
        };
        let k = p.key;
        (p, k)
    }

    /// 条件键访问（用户侧阻塞循环用）。
    pub fn key(&self) -> usize {
        self.key
    }

    /// 投递：槽空 → 存（拷贝入内核）+ Ok；槽满 → Busy；Dead → Dead。
    pub fn try_push(&self, msg: &[u8; MSG_LEN]) -> Result<(), PortError> {
        let mut st = self.inner.lock();
        if st.state == PortState::Dead {
            return Err(PortError::Dead);
        }
        if st.slot.is_some() {
            return Err(PortError::Busy);
        }
        st.slot = Some(*msg);
        Ok(())
    }

    /// 收取：槽满 → 取（拷贝出内核）+ Ok；槽空 → Busy；Dead → Dead。
    pub fn try_pull(&self) -> Result<[u8; MSG_LEN], PortError> {
        let mut st = self.inner.lock();
        if st.state == PortState::Dead {
            return Err(PortError::Dead);
        }
        match st.slot.take() {
            Some(msg) => Ok(msg),
            None => Err(PortError::Busy),
        }
    }

    /// 终止：Live → Dead。阻塞中的对端由其用户侧条件循环经 wake 感知后返回 Dead。
    pub fn shut(&self) {
        self.inner.lock().state = PortState::Dead;
    }
}

// ── port 注册表（envcall 面：句柄 → 端口）──

/// 全局 port 注册表（Level::L3，与 blocked 同级；绝不 3→3 嵌套）。
/// 条目不主动销毁（shut 置 Dead 留档；内核静态表，不涉分配器审计）。
fn ports() -> &'static SpinLock<HashMap<usize, Port>> {
    static PORTS: OnceLock<SpinLock<HashMap<usize, Port>>> = OnceLock::new();
    PORTS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// 建 port（envcall Open 入口）：注册并返回 (句柄, 条件键)。
pub fn open() -> (usize, usize) {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let (p, key) = Port::open();
    ports().lock().insert(handle, p);
    (handle, key)
}

/// 终止 port（envcall Shut 入口）：句柄未注册 → false。
pub fn shut(handle: usize) -> bool {
    let ports = ports().lock();
    let Some(p) = ports.get(&handle) else {
        return false;
    };
    let p = p.clone(); // 克隆 Arc 后放表锁——inner 锁（L3）不在表锁（L3）内获取（3→3 禁）
    drop(ports);
    p.shut();
    true
}

/// 投递（envcall Push 入口）：消息拷贝已由调用方（mail::copy_in）完成；
/// 句柄未注册视为 Dead（对端已失效）。
pub fn try_push(handle: usize, msg: &[u8; MSG_LEN]) -> Result<(), PortError> {
    let ports = ports().lock();
    let Some(p) = ports.get(&handle) else {
        return Err(PortError::Dead);
    };
    let p = p.clone();
    drop(ports);
    p.try_push(msg)
}

/// 收取（envcall Pull 入口）：拷贝出内核由调用方（mail::copy_out）完成。
pub fn try_pull(handle: usize) -> Result<[u8; MSG_LEN], PortError> {
    let ports = ports().lock();
    let Some(p) = ports.get(&handle) else {
        return Err(PortError::Dead);
    };
    let p = p.clone();
    drop(ports);
    p.try_pull()
}

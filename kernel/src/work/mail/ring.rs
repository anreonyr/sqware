// ring — 一对一共享内存邮路（mail 通道之一）：数据/索引全在用户态共享物理帧，
// 内核不搬消息（零拷贝），只提供：
//   1. 通道生命周期：open/join/shut + 状态机 Live→Dead（无 pier/quay 计数）；
//   2. 共享区登记与借用映射（帧所有权归 RingMeta，Arc 归零归还）；
//   3. 同步：wait/wake 直用调度域原语（Ucall::Wait / Ucall::Wake）——mail 不重造
//      调度器。
//
// ring = 一对一共享内存通道（两端固定：open 即生产端 + 消费端各持一端）。
// 共享区布局 = `ubi::ring`（编译期 ABI，两端同源；锁/读写弧偏移）。
// 多对一形态见 `mail::dock`（独立布局 `ubi::dock`，含 pier/quay 计数）。
//
// 视图回收：同 dock——每个 space 的共享区借用映射各占一段 user 段 VA，持有者
// 登记 `Weak<Space>` + 视图 Span；Arc<RingMeta> 归零 drop 时逐视图归还段。
//
// 生命周期（Gate 5 设计）：mail 接入点由 Task.mail 直接持有，无中间簿记——
// 任务死 → MailHolds::drop → Ring::drop → `dead` → Arc<RingMeta> 归零 →
// Meta::drop（shared 字段 drop → SharedBuf::drop → 视图 + 帧归还）。
// 注册表仅供 id→meta 弱查。
//
// 键面：ring 键 = `RING_KEY_TAG | id`（不经 WaitKey::compose——跨 team asid
// 不同，经 compose 必失配；见 envcall 的 Wait/Wake 分发）。

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use alloc::sync::{Arc, Weak};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::manager::MapError;
use crate::work::mail::SharedBuf;
use crate::work::room::scheduler::core::current_task;
use crate::work::unit::space::Space;

use ubi::ring;

/// ring 状态编码（与 ubi::ring::state 同值；内核侧语义枚举）。
///
/// 两态迁移律：open → Live；显式 shut / 对端离场 → Dead。无 Hang/Gone——
/// 一对一通道没有"pier 全 drop 仍可消费"的中间态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    /// 两端在场。
    Live,
    /// 显式 close / 对端离场。
    Dead,
}

impl RingState {
    /// 与 ubi 布局编码互转（写状态槽用）。
    pub const fn code(self) -> u8 {
        match self {
            RingState::Live => ring::state::LIVE,
            RingState::Dead => ring::state::DEAD,
        }
    }
}

/// ring 错误（R1 负码；同 ubi::ring::err）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    /// 通道已断开（close / 未登记）。
    Dead,
}

impl RingError {
    pub(crate) const fn code(self) -> isize {
        match self {
            RingError::Dead => ring::err::DEAD,
        }
    }
}

/// 共享区字段的原子视图（基于共享区基址 + 固定偏移；与 ubi::ring 布局配对）。
///
/// 内核侧只经本视图访问状态；锁/读写弧由用户侧原子协议独占（push/pull 直操作
/// 共享物理帧，零内核介入）。
#[derive(Clone, Copy)]
struct RingShared<'a> {
    base: &'a [u8],
}

impl<'a> RingShared<'a> {
    /// # Safety
    /// base 必须指向本 ring 的共享物理帧首地址（恒等映射下 VA 即 PA）。
    unsafe fn new_unchecked(base: *const u8) -> Self {
        // SAFETY: 调用方保证；读侧字段在页内，块大小 ≥ 一页。
        Self {
            base: unsafe { core::slice::from_raw_parts(base, crate::memory::PAGE_SIZE) },
        }
    }

    fn state(&self) -> &AtomicU8 {
        // SAFETY: 偏移固定 + 原子类型对齐；共享区页内，读侧安全。
        unsafe { &*(self.base.as_ptr().add(ring::OFF_STATE) as *const AtomicU8) }
    }

    /// 置 Dead（显式 close / 任何断开路径的收敛态）。
    fn dead(&self) {
        let _ = self.state().compare_exchange(
            RingState::Live.code(),
            RingState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// 写状态槽 + item_len / slots 定型字段（open 时一次）。
    fn init_layout(&self, item_len: usize, slots: usize) {
        self.state().store(RingState::Live.code(), Ordering::Release);
        // SAFETY: 固定偏移 usize 写，块内对齐。
        unsafe {
            (self.base.as_ptr().add(ring::OFF_ITEM_LEN) as *mut usize).write(item_len);
            (self.base.as_ptr().add(ring::OFF_SLOTS) as *mut usize).write(slots);
        }
        // 发布围栏：写完成后 Release 状态槽 → 用户侧 Acquire 读 state 见全字段。
        core::sync::atomic::fence(Ordering::Release);
    }
}

/// ring 内核登记元数据：全局身份 + 共享内存邮路（沿用 [`SharedBuf`]）。
pub(crate) struct RingMeta {
    /// 全局唯一 id（兼作 wait/wake 键 = `RING_KEY_TAG | id`）。
    pub(crate) id: usize,
    /// 共享内存邮路（帧 + 视图 + Drop 链）。
    pub(crate) shared: SharedBuf,
}

// SAFETY: RingMeta 经 Arc 跨任务/跨 hart 共享；shared.base 指向共享物理帧
//（恒等映射下语义 = 裸物理地址），无 Rust 对象别名，只读/原子访问——
// Send/Sync 安全。
unsafe impl Send for RingMeta {}
unsafe impl Sync for RingMeta {}

// ── 注册表（弱查：id → Weak<RingMeta>）──

/// ring 全局注册表（Level::L3；不主动清，Weak 归零后 lookup 失败）。
fn rings() -> &'static SpinLock<HashMap<usize, Weak<RingMeta>>> {
    static RINGS: OnceLock<SpinLock<HashMap<usize, Weak<RingMeta>>>> = OnceLock::new();
    RINGS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// ring 全局 id 分配器（自 1；0 保留，与 dock 独立空间）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ── 核心 API ────────────────────────────────────────────────

/// 建 ring（envcall RingOpen 入口）：a0 = item_len，a1 = slots（2 的幂）。
/// 共享区借映进**当前任务所在 space**；接入点 move 到当前 task.mail。
/// 返回 `(id, view)` —— id = 全局 ring id，view = 共享区视图基址。
///
/// # Errors
/// - `NotAligned` — item_len/slots 校验失败。
/// - `OutOfMemory` — 共享区帧不足。
/// - 映射失败（AlreadyMapped/NoRegion）向上抛。
pub fn open(space: &Arc<Space>, item_len: usize, slots: usize) -> Result<(usize, usize), MapError> {
    if item_len == 0 || slots == 0 || !slots.is_power_of_two() {
        return Err(MapError::NotAligned);
    }
    let bytes = (ring::OFF_BUFFER + item_len * slots).next_multiple_of(crate::memory::PAGE_SIZE);
    let shared = SharedBuf::allocate(bytes)?;
    // SAFETY: shared.base 在 allocate 后有效；RingShared::init_layout 写定型字段。
    unsafe {
        RingShared::new_unchecked(shared.base.as_ptr()).init_layout(item_len, slots);
    }

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let meta = Arc::new(RingMeta { id, shared });
    {
        let mut reg = rings().lock();
        reg.insert(id, Arc::downgrade(&meta));
    } // reg drop：RINGS 锁在此释放（不跨到 map_into / push）

    let view = meta.shared.map_into(space)?;

    // 接入点（单端）move 到当前 task.mail。
    let task = current_task().expect("ring::open: envcall context");
    task.mail.lock().rings.push(Ring { meta });

    Ok((id, view))
}

/// 加入已有 ring（envcall RingJoin 入口）：a0 = id。
/// 同 team（本方 space 已含该共享块映射）→ 复用既有视图；跨 team → 重新借用
/// 映射同一物理块进本方 space。接入点 move 到当前 task.mail。返回本地视图基址。
///
/// # Errors
/// - `Dead` — id 未登记（对端 ring 不存在）。
pub fn join(space: &Arc<Space>, id: usize) -> Result<usize, RingError> {
    let meta = rings()
        .lock()
        .get(&id)
        .and_then(Weak::upgrade)
        .ok_or(RingError::Dead)?;
    let view = meta.shared.map_into(space).map_err(|_| RingError::Dead)?;

    let task = current_task().expect("ring::join: envcall context");
    task.mail.lock().rings.push(Ring { meta });

    Ok(view)
}

/// 释放当前任务的 ring 接入点（envcall RingShut 入口）：查找并从 mail 移除
/// 匹配条目 —— 移除触发 `Ring::drop` → `dead` 链。
///
/// 返回：true = 释放成功；false = 当前 task 未持 `id`。
pub fn shut(id: usize) -> bool {
    let Some(task) = current_task() else { return false; };
    let mut mail = task.mail.lock();
    let Some(pos) = mail.rings.iter().position(|r| r.id() == id) else {
        return false;
    };
    mail.rings.remove(pos);
    true
}

// ── ring 接入点（mail 持有；类型即义务）────────────────────

/// ring 接入点（任务持有）。`Drop` 自动 `dead`——Arc 归零触发 Meta::drop
/// （shared 字段 drop → SharedBuf::drop → 视图 + 帧归还）。
pub struct Ring {
    meta: Arc<RingMeta>,
}

impl Ring {
    /// 全局 id（跨任务定位用）。
    pub fn id(&self) -> usize {
        self.meta.id
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: shared.base 在 Arc 持有期间恒有效；本 Drop 是 Arc 释放前最后一次访问。
        let sh = unsafe { RingShared::new_unchecked(self.meta.shared.base.as_ptr()) };
        sh.dead();
    }
}

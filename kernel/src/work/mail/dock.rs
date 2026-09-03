// dock — 多对一共享内存邮路（mail 双通道之一）：数据/索引全在用户态共享物理帧，
// 内核不搬消息（零拷贝），只提供：
//   1. 通道生命周期：open/join/shut + 状态机 Live→Hang→Gone/Dead；
//   2. 共享区登记与借用映射（帧所有权归 DockMeta，Arc 归零归还）；
//   3. 同步：wait/wake 直用调度域原语（Ucall::Wait / Ucall::Wake）——mail 不重造
//      调度器。
//
// dock = 方向性共享内存通道（多 pier 生产 / 唯一 quay 消费，词族与 port 成对）。
// 共享区布局 = `ubi::dock`（编译期 ABI，两端同源；锁/索引/计数字段偏移）。
// 一对一形态见 `mail::ring`（独立布局 `ubi::ring`，无 pier/quay 计数）。
//
// 视图回收：每个 space 的共享区借用映射各占一段 user 段 VA（空间事务内取段 +
// `borrow`），持有者登记 `Weak<Space>` + 视图 Span——Arc<DockMeta> 归零 drop 时
// 逐视图 `unmap_range` 还段（不拖住地址空间生命周期）。
//
// 生命周期（Gate 5 设计）：mail 接入点由 Task.mail 直接持有，无中间簿记——
// 任务死 → MailHolds::drop → Dock::drop → clear_quay / dec_pier
// → Arc<DockMeta> 归零 → Meta::drop（共享帧归还，视图由 SharedBuf::drop 跑）。
// 注册表仅供 id→meta 弱查。
//
// 键面：dock 键 = `DOCK_KEY_TAG | id`（不经 WaitKey::compose——跨 team asid
// 不同，经 compose 必失配；见 envcall 的 Wait/Wake 分发）。

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use alloc::sync::{Arc, Weak};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::manager::MapError;
use crate::work::mail::SharedBuf;
use crate::work::room::scheduler::core::current_task;
use crate::work::unit::space::Space;

use ubi::dock;

/// dock 状态编码（与 ubi::dock::state 同值；内核侧语义枚举）。
///
/// 四态迁移律（Gate 5 设计）：open → Live；opener Tx::drop（pier 全 drop 钩子）→
/// Hang（quay 在场）；显式 shut quay / quay Tx::drop → Dead；Hang 下余信取空 →
/// Gone（**quay 用户态钉连 CAS**——本枚举是编译期核对依据，内核侧不构造 Gone，
/// Gone 的写入发生在用户库 `user::dock` 的 pull 协议中，见 ubi::dock::state::GONE）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockState {
    /// pier_count ≥ 1 且 quay 在场。
    Live,
    /// pier 全 drop——quay 仍可取余信。
    Hang,
    /// Hang 下余信取空（quay 钉连）→ 连接自然终了。
    Gone,
    /// 显式 shut / quay 缺席。
    Dead,
}

impl DockState {
    /// 与 ubi 布局编码互转（写状态槽用）。
    pub const fn code(self) -> u8 {
        match self {
            DockState::Live => dock::state::LIVE,
            DockState::Hang => dock::state::HANG,
            DockState::Gone => dock::state::GONE,
            DockState::Dead => dock::state::DEAD,
        }
    }
}

/// dock 错误（D1 负码；同 ubi::dock::err）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockError {
    /// 通道已断开（shut / quay 缺席 / 未登记）。
    Dead,
    /// 条件不满足（满/空/quay 被占）——调用方 wait 后重试，非终结态。
    Busy,
}

impl DockError {
    pub(crate) const fn code(self) -> isize {
        match self {
            DockError::Dead => dock::err::DEAD,
            DockError::Busy => dock::err::BUSY,
        }
    }
}

/// 共享区字段的原子视图（基于共享区基址 + 固定偏移；与 ubi::dock 布局配对）。
///
/// 内核侧只经本视图访问计数/状态（生命周期迁移）；锁/索引弧由用户侧原子协议
/// 独占（push/pull 直操作共享物理帧，零内核介入）。
#[derive(Clone, Copy)]
struct DockShared<'a> {
    base: &'a [u8],
}

impl<'a> DockShared<'a> {
    /// # Safety
    /// base 必须指向本 dock 的共享物理帧首地址（恒等映射下 VA 即 PA）。
    unsafe fn new_unchecked(base: *const u8) -> Self {
        // SAFETY: 调用方保证；读侧字段在页内，块大小 ≥ 一页。
        Self {
            base: unsafe { core::slice::from_raw_parts(base, crate::memory::PAGE_SIZE) },
        }
    }

    fn state(&self) -> &AtomicU8 {
        // SAFETY: 偏移固定 + 原子类型对齐；共享区页内，读侧安全。
        unsafe { &*(self.base.as_ptr().add(dock::OFF_STATE) as *const AtomicU8) }
    }
    fn pier_count(&self) -> &AtomicUsize {
        // SAFETY: 同 state。
        unsafe { &*(self.base.as_ptr().add(dock::OFF_PIER_COUNT) as *const AtomicUsize) }
    }
    fn quay(&self) -> &core::sync::atomic::AtomicBool {
        // SAFETY: 同 state。
        unsafe { &*(self.base.as_ptr().add(dock::OFF_QUAY) as *const core::sync::atomic::AtomicBool) }
    }

    /// pier 计数 −1；归零且 quay 缺席 → Live→Hang（CAS）。
    fn dec_pier(&self) -> usize {
        let n = self
            .pier_count()
            .fetch_sub(1, Ordering::AcqRel)
            .saturating_sub(1);
        if n == 0
            && !self.quay().load(Ordering::Acquire)
            && self
                .state()
                .compare_exchange(
                    DockState::Live.code(),
                    DockState::Hang.code(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        {
            // Live→Hang 成功（solo 迁移者）。
        }
        n
    }

    /// quay 离场：quay 位清 + 状态 → Dead（Live→Dead 或 Hang→Dead 皆可）。
    fn clear_quay(&self) {
        self.quay().store(false, Ordering::Release);
        let _ = self.state().compare_exchange(
            DockState::Live.code(),
            DockState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let _ = self.state().compare_exchange(
            DockState::Hang.code(),
            DockState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// 写状态槽 + item_len / slots 定型字段（open 时一次）。
    fn init_layout(&self, item_len: usize, slots: usize) {
        self.state().store(DockState::Live.code(), Ordering::Release);
        self.quay().store(true, Ordering::Release);
        // SAFETY: 固定偏移 usize 写，块内对齐。
        unsafe {
            (self.base.as_ptr().add(dock::OFF_ITEM_LEN) as *mut usize).write(item_len);
            (self.base.as_ptr().add(dock::OFF_SLOTS) as *mut usize).write(slots);
        }
        // 发布围栏：写完成后 Release 状态槽 → 用户侧 Acquire 读 state 见全字段。
        core::sync::atomic::fence(Ordering::Release);
    }
}

/// dock 内核登记元数据：全局身份 + 共享内存邮路（沿用 [`SharedBuf`]）。
///
/// 不显式 `impl Drop`——`shared: SharedBuf` 字段 drop 时触发 `SharedBuf::drop`
///（视图回收 + 帧归还）。`Dock::drop` 负责 Dock 特有状态字段（state/pier/quay
/// 原子槽）的离场迁移，二者解耦。
pub(crate) struct DockMeta {
    /// 全局唯一 id（兼作 wait/wake 键 = `DOCK_KEY_TAG | id`）。
    pub(crate) id: usize,
    /// 共享内存邮路（帧 + 视图 + Drop 链）。
    pub(crate) shared: SharedBuf,
}

// SAFETY: DockMeta 经 Arc 跨任务/跨 hart 共享；shared.base 指向共享物理帧
//（恒等映射下语义 = 裸物理地址），无 Rust 对象别名，只读/原子访问——
// Send/Sync 安全（读写同步由共享区内原子协议保证）。
unsafe impl Send for DockMeta {}
unsafe impl Sync for DockMeta {}

// ── 注册表（弱查：id → Weak<DockMeta>）──

/// dock 全局注册表（Level::L3；不主动清，Weak 归零后 lookup 失败）。
fn docks() -> &'static SpinLock<HashMap<usize, Weak<DockMeta>>> {
    static DOCKS: OnceLock<SpinLock<HashMap<usize, Weak<DockMeta>>>> = OnceLock::new();
    DOCKS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// dock 全局 id 分配器（自 1；0 保留）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ── 核心 API ────────────────────────────────────────────────

/// 建 dock（envcall DockOpen 入口）：a0 = item_len，a1 = slots（2 的幂）。
/// 共享区借映进**当前任务所在 space**；接入点（Rx + Tx 两端）move 到当前
/// task.mail。返回 `(id, view)` —— id = 全局 dock id，view = 共享区视图基址。
///
/// # Errors
/// - `NotAligned` — item_len/slots 校验失败。
/// - `OutOfMemory` — 共享区帧不足。
/// - 映射失败（AlreadyMapped/NoRegion）向上抛。
pub fn open(space: &Arc<Space>, item_len: usize, slots: usize) -> Result<(usize, usize), MapError> {
    if item_len == 0 || slots == 0 || !slots.is_power_of_two() {
        return Err(MapError::NotAligned);
    }
    let bytes = (dock::OFF_BUFFER + item_len * slots).next_multiple_of(crate::memory::PAGE_SIZE);
    let shared = SharedBuf::allocate(bytes)?;
    // SAFETY: shared.base 在 allocate 后有效；DockShared::init_layout 写定型字段。
    unsafe {
        DockShared::new_unchecked(shared.base.as_ptr()).init_layout(item_len, slots);
    }

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let meta = Arc::new(DockMeta { id, shared });
    {
        let mut reg = docks().lock();
        reg.insert(id, Arc::downgrade(&meta));
    } // reg drop：DOCKS 锁在此释放（不跨到 map_into / push）

    let view = meta.shared.map_into(space)?;

    // 创建者持两端——接入点 move 到当前 task.mail。
    let task = current_task().expect("dock::open: envcall context");
    task.mail.lock().docks.push(Dock::Bundle {
        meta: meta.clone(),
    });

    Ok((id, view))
}

/// 加入已有 dock（envcall DockJoin 入口）：a0 = id，a1 = side（0 = Pier / 1 = Quay）。
/// 同 team（本方 space 已含该共享块映射）→ 复用既有视图；跨 team → 重新借用
/// 映射同一物理块进本方 space。Quay 侧 CAS 在场（已被占 → Busy）。
///
/// 接入点（单端）move 到当前 task.mail。返回本地视图基址。
///
/// # Errors
/// - `Dead` — id 未登记（对端 dock 不存在）。
/// - `Busy` — quay 已被占用。
pub fn join(space: &Arc<Space>, id: usize, side: usize) -> Result<usize, DockError> {
    let meta = docks()
        .lock()
        .get(&id)
        .and_then(Weak::upgrade)
        .ok_or(DockError::Dead)?;
    if side == ubi::dock::side::QUAY {
        // 唯一性 CAS：quay 在场位。open 方已持 quay → 跨任务 join as quay 需
        // open 方先 shut。
        let sh = unsafe { DockShared::new_unchecked(meta.shared.base.as_ptr()) };
        if sh
            .quay()
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DockError::Busy);
        }
    }
    let view = meta.shared.map_into(space).map_err(|_| DockError::Dead)?;

    let task = current_task().expect("dock::join: envcall context");
    let dock = match side {
        ubi::dock::side::PIER => Dock::Pier { meta },
        ubi::dock::side::QUAY => Dock::Quay { meta },
        _ => return Err(DockError::Dead),
    };
    task.mail.lock().docks.push(dock);

    Ok(view)
}

/// 释放当前任务的接入点（envcall DockShut 入口）：查找并从 mail 移除匹配条目
/// —— 移除触发 `Dock::drop` → `clear_quay / dec_pier` 链。
///
/// 返回：true = 释放成功；false = 当前 task 未持 `id`。
pub fn shut(id: usize) -> bool {
    let Some(task) = current_task() else { return false; };
    let mut mail = task.mail.lock();
    let Some(pos) = mail.docks.iter().position(|d| d.id() == id) else {
        return false;
    };
    mail.docks.remove(pos);
    true
}

// ── dock 接入点（mail 持有；类型即义务）────────────────────

/// dock 接入点（任务持有）：创建者两端（Bundle）或单端加入者（Pier / Quay）。
///
/// Drop 自动透传：Bundle 字段 drop（meta Arc 递减，最后一份时 Meta::drop 跑——
/// shared 字段 drop → SharedBuf::drop → 视图回收 + 帧归还）；Pier::drop 触发
/// `dec_pier`；Quay::drop 触发 `clear_quay`。
pub enum Dock {
    /// 创建者两端——open 返回此变体。
    Bundle { meta: Arc<DockMeta> },
    /// 加入者持发送端（pier）。
    Pier { meta: Arc<DockMeta> },
    /// 加入者持接收端（quay）。
    Quay { meta: Arc<DockMeta> },
}

impl Dock {
    /// 全局 id（跨任务定位用）。
    pub fn id(&self) -> usize {
        match self {
            Dock::Bundle { meta } => meta.id,
            Dock::Pier { meta } => meta.id,
            Dock::Quay { meta } => meta.id,
        }
    }
}

impl Drop for Dock {
    fn drop(&mut self) {
        let meta = match self {
            Dock::Bundle { meta } => meta,
            Dock::Pier { meta } => meta,
            Dock::Quay { meta } => meta,
        };
        // SAFETY: shared.base 在 Arc 持有期间恒有效；本 Drop 是 Arc 释放前最后一次访问。
        let sh = unsafe { DockShared::new_unchecked(meta.shared.base.as_ptr()) };
        match self {
            Dock::Bundle { .. } => {
                // 创建者持两端——两端 drop 都跑一次（saturating 处理重复）。
                let _ = sh.dec_pier();
                sh.clear_quay();
            }
            Dock::Pier { .. } => {
                let _ = sh.dec_pier();
            }
            Dock::Quay { .. } => sh.clear_quay(),
        }
    }
}

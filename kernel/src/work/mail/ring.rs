// ring — dock 共享内存邮路（mail 双通道之一）：数据/索引全在用户态共享物理帧，
// 内核不搬消息（零拷贝），只提供：
//   1. 通道生命周期：open/join/shut/clone/drop + 状态机 Live→Hang→Gone/Dead；
//   2. 共享区登记与借用映射（帧所有权归 DockMeta，Arc 归零归还）；
//   3. 同步：wait/wake 直用调度域原语（Ucall::Wait / Ucall::Wake）——mail 不重造
//      调度器。
//
// dock = 方向性共享内存通道（多 pier 生产 / 唯一 quay 消费，词族与 port 成对）。
// 共享区布局 = `ubi::dock`（编译期 ABI，两端同源；锁/索引/计数字段偏移）。
//
// 键面：dock 键 = `DOCK_KEY_TAG | id`（不经 WaitKey::compose——跨 team asid
// 不同，经 compose 必失配；见 envcall 的 Wait/Wake 分发）。

use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use alloc::sync::Arc;
use alloc::vec::Vec;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{MapKind, Space};

use ubi::dock;

/// dock 状态编码（与 ubi::dock::state 同值；内核侧语义枚举）。
///
/// 四态迁移律（第 4 关签名批准）：open → Live；pier 全 drop（Arc 递减钩子）→
/// Hang；显式 shut / quay 离场 → Dead；Hang 下余信取空 → Gone（**quay 用户态
/// 钉连 CAS**——本枚举是编译期核对依据，内核侧不构造 Gone，Gone 的写入发生在
/// 用户库 `user::dock` 的 pull 协议中，见 ubi::dock::state::GONE）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    /// pier_count ≥ 1 且 quay 在场。
    Live,
    /// pier 全 drop——quay 仍可取余信。
    Hang,
    /// Hang 下余信取空（quay 钉连）→ 连接自然终了。
    Gone,
    /// 显式 shut / quay 缺席。
    Dead,
}

impl RingState {
    /// 与 ubi 布局编码互转（写状态槽用）。
    pub const fn code(self) -> u8 {
        match self {
            RingState::Live => dock::state::LIVE,
            RingState::Hang => dock::state::HANG,
            RingState::Gone => dock::state::GONE,
            RingState::Dead => dock::state::DEAD,
        }
    }

    /// Hang → Gone：quay 在取空余信后钉连（用户库执行的 CAS；本方法为契约
    /// 镜像——用户侧 `DockState` 内联等价逻辑，此处仅作迁移表文档）。
    #[allow(dead_code)] // 契约文档：实现侧在 user::dock（本枚举是编译期核对依据）
    pub const fn hang_to_gone(&self) -> Option<RingState> {
        match self {
            RingState::Hang => Some(RingState::Gone),
            _ => None,
        }
    }

    /// 是否仍可消费：Live / Hang 可取（Gone / Dead 断开）。
    #[allow(dead_code)] // 契约文档：消费侧语义在用户库 pull 协议
    pub const fn pullable(&self) -> bool {
        matches!(self, RingState::Live | RingState::Hang)
    }
}

/// dock 错误（D1 负码；同 ubi::dock::err）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockError {
    /// 通道已断开（shut / quay 缺席 / 未登记）。
    Dead,
    /// 条件不满足（满/空/quay 被占）——调用方 wait 后重试，非终结态。
    Busy,
    /// Hang 下余信取空后钉连——连接自然终了（用户库 pull 协议产生；读音在
    /// envcall 边界经 [`DockError::code`] 落 a0，此处无内核构造点）。
    #[allow(dead_code)] // 契约文档：Gone 的构造在 user::dock（用户态 CAS 钉连）
    Gone,
}

impl DockError {
    pub(crate) const fn code(self) -> isize {
        match self {
            DockError::Dead => dock::err::DEAD,
            DockError::Busy => dock::err::BUSY,
            DockError::Gone => dock::err::GONE,
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
    /// 共享区首字节（物理连续块）；调用方保证 base 有效且生命周期由 Arc 保障。
    ///
    /// # Safety
    /// base 必须指向本 dock 的共享物理帧首地址（恒等映射下 VA 即 PA）。
    unsafe fn new_unchecked(base: *const u8) -> Self {
        // SAFETY: 调用方保证；读侧字段在页内，块大小 ≥ 一页。
        Self {
            base: unsafe { core::slice::from_raw_parts(base, PAGE_SIZE) },
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
    fn quay(&self) -> &AtomicBool {
        // SAFETY: 同 state。
        unsafe { &*(self.base.as_ptr().add(dock::OFF_QUAY) as *const AtomicBool) }
    }

    /// pier 计数 −1；归零且 quay 缺席 → Live→Hang（CAS）。返回迁移后的计数。
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
                    RingState::Live.code(),
                    RingState::Hang.code(),
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
            RingState::Live.code(),
            RingState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let _ = self.state().compare_exchange(
            RingState::Hang.code(),
            RingState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// 置 Dead（显式 shut / 任何断开路径的收敛态）。
    fn dead(&self) {
        let _ = self.state().compare_exchange(
            RingState::Live.code(),
            RingState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let _ = self.state().compare_exchange(
            RingState::Hang.code(),
            RingState::Dead.code(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// dock 内核登记元数据：共享区所有权（Arc 归零归还帧）+ 全局身份。
struct DockMeta {
    /// 全局唯一 id（兼作 wait/wake 键 = `DOCK_KEY_TAG | id`）。
    id: usize,
    /// 共享区物理连续块（首地址 + 字节长），双端借用映射此块。
    base: NonNull<u8>,
    bytes: usize,
}

// SAFETY: DockMeta 经 Arc 跨任务/跨 hart 共享；base 指向共享物理帧（恒等映射
// 下语义 = 裸物理地址），无 Rust 对象别名，只读/原子访问——Send/Sync 安全
//（读写同步由共享区内原子协议保证）。
unsafe impl Send for DockMeta {}
unsafe impl Sync for DockMeta {}

impl DockMeta {
    /// 从 frame 分配器分配共享区（单次 allocate = 物理连续），清零后初始化状态。
    fn allocate_shared(item_len: usize, slots: usize) -> Result<(NonNull<u8>, usize), MapError> {
        let bytes = (dock::OFF_BUFFER + item_len * slots).next_multiple_of(PAGE_SIZE);
        let layout = core::alloc::Layout::from_size_align(bytes, PAGE_SIZE)
            .map_err(|_| MapError::NotAligned)?;
        let ptr = frame::allocator()
            .allocate(layout)
            .map_err(|_| MapError::OutOfMemory)?;
        // SAFETY: 分配返回的切片指针非空；转成字节指针（长度无关，仅取首址）。
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        // SAFETY: 刚分配的合法块（fresh），全长度可写；清零初始化共享区。
        unsafe { core::ptr::write_bytes(base.as_ptr(), 0, bytes) };
        Ok((base, bytes))
    }

    /// 初始化状态槽与只读字段（open 定型后写一次）。
    fn init_state(&self, item_len: usize, slots: usize) {
        // SAFETY: base 为本 dock 共享区（恒等映射直指物理块）；布局契约。
        let sh = unsafe { DockShared::new_unchecked(self.base.as_ptr()) };
        sh.state().store(RingState::Live.code(), Ordering::Release);
        sh.quay().store(true, Ordering::Release); // open 即两端同持（pier + quay 在场）
        // 供用户侧读的只读字段：item_len / slots（open 定型后只读；写先于
        // 状态槽 Release 发布——fence 见下）。
        // SAFETY: 固定偏移 usize 写，块内对齐。
        unsafe {
            (self.base.as_ptr().add(dock::OFF_ITEM_LEN) as *mut usize).write(item_len);
            (self.base.as_ptr().add(dock::OFF_SLOTS) as *mut usize).write(slots);
        }
        // 发布围栏：写完成后 Release 状态槽 → 用户侧 Acquire 读 state 见全字段。
        core::sync::atomic::fence(Ordering::Release);
    }
}

impl Drop for DockMeta {
    fn drop(&mut self) {
        // 共享区归还 frame 池（Arc 归零 = 双端全 drop → B2 钩子）。
        let layout = core::alloc::Layout::from_size_align(self.bytes, PAGE_SIZE)
            .expect("dock frame layout valid");
        // SAFETY: base/bytes 与 allocate 时同源（Layout 可复原）。
        unsafe {
            frame::allocator().deallocate(self.base, layout);
        }
    }
}

// ── dock 注册表（envcall 面：id → Arc<DockMeta>；与 port 注册表同式）──

/// dock 全局注册表（Level::L3；不主动销毁，全 drop 后除名）。
fn docks() -> &'static SpinLock<HashMap<usize, Arc<DockMeta>>> {
    static DOCKS: OnceLock<SpinLock<HashMap<usize, Arc<DockMeta>>>> = OnceLock::new();
    DOCKS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// dock 全局 id 分配器（自 1；0 保留）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

fn dock(id: usize) -> Option<Arc<DockMeta>> {
    docks().lock().get(&id).cloned()
}

/// 全部引用消亡 → 注册表除名（Arc 归零触发 DockMeta::drop → 帧归还）。
fn maybe_purge(meta: &Arc<DockMeta>) {
    if Arc::strong_count(meta) > 1 {
        return;
    }
    docks().lock().remove(&meta.id);
}

// ── 任务名下 dock 引用（B3 钩子：clear 逐条递减，与显式 drop_end 同路径）──

fn task_docks() -> &'static SpinLock<HashMap<usize, Vec<(usize, usize)>>> {
    static TASK_DOCKS: OnceLock<SpinLock<HashMap<usize, Vec<(usize, usize)>>>> = OnceLock::new();
    TASK_DOCKS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

fn current_task_id() -> usize {
    crate::work::room::conductor::core::ident()
        .map(|i| i.id())
        .unwrap_or(usize::MAX)
}

fn register_task_dock(id: usize, side: usize) {
    let task_id = current_task_id();
    task_docks()
        .lock()
        .entry(task_id)
        .or_default()
        .push((id, side));
}

fn unregister_task_dock(id: usize, side: usize) {
    let task_id = current_task_id();
    let mut table = task_docks().lock();
    if let Some(v) = table.get_mut(&task_id) {
        v.retain(|&(i, s)| !(i == id && s == side));
    }
}

/// 任务退出钩子（conductor::core::clear 调用）：该任务名下全部 dock 引用逐条
/// 递减（等价多个显式 drop_end），并清理登记。
pub(crate) fn task_exit(task_id: usize) {
    let entries: Vec<(usize, usize)> = task_docks().lock().remove(&task_id).unwrap_or_default();
    for (id, side) in entries {
        let Some(meta) = dock(id) else {
            continue;
        };
        match side {
            dock::side::PIER => {
                // SAFETY: base 为本 dock 共享区。
                let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
                sh.dec_pier();
                maybe_purge(&meta);
            }
            dock::side::QUAY => {
                // SAFETY: base 为本 dock 共享区。
                let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
                sh.clear_quay();
                maybe_purge(&meta);
            }
            _ => {}
        }
    }
}

// ── 共享区借用映射 ──────────────────────────────────────────

/// 把共享物理块借用映射进 `space`（帧空 = 借用；VA 出自由段）。
/// 与 trampoline 借用映射同式（Space::map 无帧）：PTE 直指共享物理块，帧所有权
/// 留 DockMeta（Arc 归零归还）。视图随 Space 常数 Map drop 消失。
fn map_shared(space: &Space, meta: &DockMeta, bytes: usize) -> Result<usize, MapError> {
    let size = bytes.next_multiple_of(PAGE_SIZE);
    // free 段访问器（SpaceInner 激活自由段时留档；dock 属非窗口借用方）
    let va = space.with(|inner| {
        let free = inner.free.as_ref().ok_or(MapError::NoRegion)?;
        let base = free.allocate(size).map_err(|_| MapError::OutOfMemory)?;
        Ok(VirtAddr::from_raw(base))
    })?;
    space.map(
        va,
        PhysAddr::from_raw(meta.base.as_ptr() as usize),
        size,
        PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D,
        MapKind::Anonymous,
        Vec::new(), // 借用：无帧
    )?;
    Ok(va.as_usize())
}

// ── 适配面：envcall 五路 ────────────────────────────────────

/// 建 dock（envcall RingOpen 入口）：a0 = item_len，a1 = slots（2 的幂）。
/// 返回 (dock id, 视图基址)。视图 = 共享区借用映射进**当前任务所在 space**
/// （同 team 双端同空间；跨 team 由对端 join 再借同一物理块）。
///
/// # Errors
/// - `NotAligned` — item_len/slots 校验失败。
/// - `OutOfMemory` — 共享区帧不足。
/// - 映射失败（AlreadyMapped/NoRegion）向上抛。
pub fn open(space: &Space, item_len: usize, slots: usize) -> Result<(usize, usize), MapError> {
    if item_len == 0 || slots == 0 || !slots.is_power_of_two() {
        return Err(MapError::NotAligned);
    }
    let (base, bytes) = DockMeta::allocate_shared(item_len, slots)?;
    let meta = Arc::new(DockMeta {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        base,
        bytes,
    });
    meta.init_state(item_len, slots);
    docks().lock().insert(meta.id, meta.clone());

    let va = map_shared(space, &meta, bytes)?;

    // open 方自持 pier + quay 两引用（Drop 钩子逐一递减）。
    register_task_dock(meta.id, dock::side::PIER);
    register_task_dock(meta.id, dock::side::QUAY);

    Ok((meta.id, va))
}

/// 加入已有 dock（envcall RingJoin 入口）：a0 = id，a1 = side。
/// 同 team（本方 space 已含该共享块映射）→ 复用既有视图；跨 team → 重新借用
/// 映射同一物理块进本方 space。Quay 侧 CAS 在场（已被占 → Busy）。
///
/// 返回本地视图基址。
///
/// # Errors
/// - `Dead` — id 未登记（对端 dock 不存在）。
/// - `Busy` — quay 已被占用。
pub fn join(space: &Space, id: usize, side: usize) -> Result<usize, DockError> {
    let meta = dock(id).ok_or(DockError::Dead)?;
    if side == dock::side::QUAY {
        // 唯一性 CAS：quay 在场位。open 方已持 quay（同 team joiner 是 open 方
        // 另一任务）→ 首版语义：quay 只能一份，跨 team 接 quay 需 open 方先 drop。
        let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
        if sh
            .quay()
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DockError::Busy);
        }
    }
    let va = map_shared(space, &meta, meta.bytes).map_err(|_| DockError::Dead)?;
    register_task_dock(id, side);
    Ok(va)
}

/// 终止 dock（envcall RingShut 入口）：置 Dead（对端感知断开）。
pub fn shut(id: usize) -> bool {
    let Some(meta) = dock(id) else {
        return false;
    };
    // SAFETY: base 为本 dock 共享区。
    let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
    sh.dead();
    true
}

/// 复制生产端（envcall RingClone 入口）：pier 计数 +1（登记给当前任务）。
pub fn clone_pier(id: usize) -> bool {
    let Some(meta) = dock(id) else {
        return false;
    };
    // SAFETY: base 为本 dock 共享区。
    let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
    sh.pier_count().fetch_add(1, Ordering::AcqRel);
    register_task_dock(id, dock::side::PIER);
    true
}

/// 释放一端（envcall RingDrop 入口）：pier −1（归零 → Hang）/ quay 离场（→ Dead）。
pub fn drop_end(id: usize, side: usize) -> bool {
    let Some(meta) = dock(id) else {
        return false;
    };
    // SAFETY: base 为本 dock 共享区。
    let sh = unsafe { DockShared::new_unchecked(meta.base.as_ptr()) };
    match side {
        dock::side::PIER => {
            sh.dec_pier();
        }
        dock::side::QUAY => {
            sh.clear_quay();
        }
        _ => return false,
    }
    unregister_task_dock(id, side);
    maybe_purge(&meta);
    true
}

// ring — 一对一共享内存邮路（mail 通道之一）：数据/索引全在用户态共享物理帧，
// 内核不搬消息（零拷贝），只提供：
//   1. 通道生命周期：open/join/close + 状态机 Live→Dead（无 pier/quay 计数）；
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
// 键面：ring 键 = `RING_KEY_TAG | id`（不经 WaitKey::compose——跨 team asid
// 不同，经 compose 必失配；见 envcall 的 Wait/Wake 分发）。

use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::PhysAddr;
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{Seg, Space, Span};

use ubi::ring;

/// ring 状态编码（与 ubi::ring::state 同值；内核侧语义枚举）。
///
/// 两态迁移律：open → Live；显式 close / 对端离场 → Dead。无 Hang/Gone——
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

    /// 是否仍可消费。
    #[allow(dead_code)] // 契约文档：消费侧语义在用户库 pull 协议
    pub const fn pullable(self) -> bool {
        matches!(self, RingState::Live)
    }
}

/// ring 错误（R1 负码；同 ubi::ring::err）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    /// 通道已断开（close / 未登记）。
    Dead,
    /// 条件不满足（满/空）——调用方 wait 后重试，非终结态。
    #[allow(dead_code)] // 契约文档：构造在用户库 user::ring（锁内满/空判据）
    Busy,
}

impl RingError {
    pub(crate) const fn code(self) -> isize {
        match self {
            RingError::Dead => ring::err::DEAD,
            RingError::Busy => ring::err::BUSY,
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
    /// 共享区首字节（物理连续块）；调用方保证 base 有效且生命周期由 Arc 保障。
    ///
    /// # Safety
    /// base 必须指向本 ring 的共享物理帧首地址（恒等映射下 VA 即 PA）。
    unsafe fn new_unchecked(base: *const u8) -> Self {
        // SAFETY: 调用方保证；读侧字段在页内，块大小 ≥ 一页。
        Self {
            base: unsafe { core::slice::from_raw_parts(base, PAGE_SIZE) },
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
}

/// ring 内核登记元数据：共享区所有权（Arc 归零归还帧）+ 全局身份 + 视图登记。
struct RingMeta {
    /// 全局唯一 id（兼作 wait/wake 键 = `RING_KEY_TAG | id`）。
    id: usize,
    /// 共享区物理连续块（首地址 + 字节长），双端借用映射此块。
    base: NonNull<u8>,
    bytes: usize,
    /// 各持有 space 的视图（弱引用 + 段区间）——drop 时逐视图归还 user 段。
    /// 同 dock：弱引用不拖住地址空间生命周期。锁序：exempt（不参与层级校验，
    /// map_shared 可能在持 RINGS 锁时取它）。
    views: SpinLock<Vec<(Weak<Space>, Span)>>,
}

// SAFETY: RingMeta 经 Arc 跨任务/跨 hart 共享；base 指向共享物理帧（恒等映射
// 下语义 = 裸物理地址），无 Rust 对象别名，只读/原子访问——Send/Sync 安全。
unsafe impl Send for RingMeta {}
unsafe impl Sync for RingMeta {}

impl RingMeta {
    /// 从 frame 分配器分配共享区（单次 allocate = 物理连续），清零后初始化状态。
    fn allocate_shared(item_len: usize, slots: usize) -> Result<(NonNull<u8>, usize), MapError> {
        let bytes = (ring::OFF_BUFFER + item_len * slots).next_multiple_of(PAGE_SIZE);
        let layout = core::alloc::Layout::from_size_align(bytes, PAGE_SIZE)
            .map_err(|_| MapError::NotAligned)?;
        // 类别 = Task：共享区帧归通道元数据（任务生命周期）——关机归零。
        let ptr = crate::memory::allocator::fence::alloc_frame(
            crate::memory::allocator::fence::FrameClass::Task,
        )
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
        // SAFETY: base 为本 ring 共享区（恒等映射直指物理块）；布局契约。
        let sh = unsafe { RingShared::new_unchecked(self.base.as_ptr()) };
        sh.state().store(RingState::Live.code(), Ordering::Release);
        // 供用户侧读的只读字段：item_len / slots（open 定型后只读；写先于
        // 状态槽 Release 发布——fence 见下）。
        // SAFETY: 固定偏移 usize 写，块内对齐。
        unsafe {
            (self.base.as_ptr().add(ring::OFF_ITEM_LEN) as *mut usize).write(item_len);
            (self.base.as_ptr().add(ring::OFF_SLOTS) as *mut usize).write(slots);
        }
        // 发布围栏：写完成后 Release 状态槽 → 用户侧 Acquire 读 state 见全字段。
        core::sync::atomic::fence(Ordering::Release);
    }
}

impl Drop for RingMeta {
    fn drop(&mut self) {
        // 1. 逐视图归还 user 段（取空后放锁再回收——Space::release 经 Space 锁，
        //    不得在 L3 持锁内调用）。
        let views: Vec<(Weak<Space>, Span)> = core::mem::take(&mut *self.views.lock());
        for (weak, span) in views {
            if let Some(space) = weak.upgrade() {
                space.release(span);
            }
        }
        // 2. 共享区归还 frame 池（Arc 归零 = 双端全 drop → B2 钩子）。
        let layout = core::alloc::Layout::from_size_align(self.bytes, PAGE_SIZE)
            .expect("ring frame layout valid");
        // SAFETY: base/bytes 与 allocate 时同源（Layout 可复原）。
        unsafe {
            frame::allocator().deallocate(self.base, layout);
        }
    }
}

// ── ring 注册表（envcall 面：id → Arc<RingMeta>；与 dock 注册表同式）──

/// ring 全局注册表（Level::L3；不主动销毁，全 drop 后除名）。
fn rings() -> &'static SpinLock<HashMap<usize, Arc<RingMeta>>> {
    static RINGS: OnceLock<SpinLock<HashMap<usize, Arc<RingMeta>>>> = OnceLock::new();
    RINGS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

/// ring 全局 id 分配器（自 1；0 保留，与 dock 独立空间）。
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

fn ring(id: usize) -> Option<Arc<RingMeta>> {
    rings().lock().get(&id).cloned()
}

/// 全部引用消亡 → 注册表除名（Arc 归零触发 RingMeta::drop → 帧归还）。
fn count_ring_refs(id: usize) -> usize {
    task_rings()
        .lock()
        .values()
        .map(|v| v.iter().filter(|&&i| i == id).count())
        .sum()
}

/// 关机清理（tie::halt 调用）：同 dock::shutdown——drain 出全部 Arc 后放锁再
/// drop（drop 内 space.release 经 Space 锁，持 L3 时 drop 会层级下降违规）。
pub(crate) fn shutdown() {
    let metas: Vec<Arc<RingMeta>> = {
        let mut reg = rings().lock();
        reg.drain().map(|(_, v)| v).collect()
    };
    drop(metas);
}

impl RingMeta {
    /// 全部视图 space 已 drop（Weak upgrade 全失败）→ 无空间再访问共享区。
    fn views_all_dead(&self) -> bool {
        let views = self.views.lock();
        views.iter().all(|(w, _)| w.upgrade().is_none())
    }
}

/// 登记归零 + 无存活视图 space → 注册表除名（同 dock::purge_if_unreferenced）。
///
/// 不依赖 Arc strong_count（task_rings 不持 Arc）。**两条件缺一不可**：主任务
/// exit 时登记归零但子任务仍持视图 space（同 team 未 drop）→ 不 purge，子任务
/// 可继续访问共享区；space 全 drop 后才真正释放共享区帧。
fn purge_if_unreferenced(meta: &Arc<RingMeta>) {
    if count_ring_refs(meta.id) == 0 && meta.views_all_dead() {
        rings().lock().remove(&meta.id);
    }
}

// ── 任务名下 ring 引用（clear 钩子：逐条递减，与显式 drop 同路径）──

fn task_rings() -> &'static SpinLock<HashMap<usize, Vec<usize>>> {
    static TASK_RINGS: OnceLock<SpinLock<HashMap<usize, Vec<usize>>>> = OnceLock::new();
    TASK_RINGS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

fn current_task_id() -> usize {
    crate::work::room::conductor::core::ident()
        .map(|i| i.id())
        .unwrap_or(usize::MAX)
}

fn register_task_ring(id: usize) {
    let task_id = current_task_id();
    task_rings()
        .lock()
        .entry(task_id)
        .or_default()
        .push(id);
}

#[allow(dead_code)] // 契约文档：一对一 close 为全局操作，端级 unregister 预留
fn unregister_task_ring(id: usize) {
    let task_id = current_task_id();
    let mut table = task_rings().lock();
    if let Some(v) = table.get_mut(&task_id) {
        v.retain(|&i| i != id);
    }
}

/// 任务退出钩子（conductor::core::clear 调用）：该任务名下全部 ring 引用逐条
/// 关闭（等价多个显式 drop），并清理登记。
pub(crate) fn task_exit(task_id: usize) {
    let entries: Vec<usize> = task_rings().lock().remove(&task_id).unwrap_or_default();
    let mut metas = Vec::new();
    for id in entries {
        let Some(meta) = ring(id) else {
            continue;
        };
        // SAFETY: base 为本 ring 共享区。
        let sh = unsafe { RingShared::new_unchecked(meta.base.as_ptr()) };
        sh.dead();
        metas.push(meta);
    }
    // 全部关闭完成后，检查各 ring 是否还有别的任务持引用 / 存活视图空间 → 无则
    // 除名（按 id 去重——同一 ring 可能多条登记）。
    let mut seen = Vec::new();
    for meta in metas {
        if !seen.contains(&meta.id) {
            seen.push(meta.id);
            purge_if_unreferenced(&meta);
        }
    }
}

// ── 共享区借用映射 ──────────────────────────────────────────

/// 把共享物理块借用映射进 `space`（帧空 = 借用；VA 出 user 段），并登记视图
/// 到 `meta`（回收身份）。同一 space 重复映射 → 复用既有视图（不重复取段）。
/// 与 dock 同式：PTE 直指共享物理块，帧所有权留 RingMeta（Arc 归零归还）。
fn map_shared(space: &Arc<Space>, meta: &RingMeta, bytes: usize) -> Result<usize, MapError> {
    {
        let views = meta.views.lock();
        if let Some((_, span)) = views
            .iter()
            .find(|(w, _)| w.upgrade().map_or(false, |s| Arc::ptr_eq(&s, space)))
        {
            return Ok(span.va.as_usize());
        }
    }
    let size = bytes.next_multiple_of(PAGE_SIZE);
    let va = space.alloc(Seg::User, size)?;
    space.map(
        va,
        PhysAddr::from_raw(meta.base.as_ptr() as usize),
        size,
        PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D,
        Vec::new(), // 借用：无帧
    )?;
    meta.views.lock().push((
        Arc::downgrade(space),
        Span::new(Seg::User, va, size, None),
    ));
    Ok(va.as_usize())
}

// ── 适配面：envcall 三路 ────────────────────────────────────

/// 建 ring（envcall RingOpen 入口）：a0 = item_len，a1 = slots（2 的幂）。
/// 返回 (ring id, 视图基址)。视图 = 共享区借用映射进**当前任务所在 space**。
///
/// # Errors
/// - `NotAligned` — item_len/slots 校验失败。
/// - `OutOfMemory` — 共享区帧不足。
/// - 映射失败（AlreadyMapped/NoRegion）向上抛。
pub fn open(space: &Arc<Space>, item_len: usize, slots: usize) -> Result<(usize, usize), MapError> {
    if item_len == 0 || slots == 0 || !slots.is_power_of_two() {
        return Err(MapError::NotAligned);
    }
    let (base, bytes) = RingMeta::allocate_shared(item_len, slots)?;
    let meta = Arc::new(RingMeta {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        base,
        bytes,
        views: SpinLock::new(Vec::new()),
    });
    meta.init_state(item_len, slots);
    {
        let mut reg = rings().lock();
        reg.insert(meta.id, meta.clone());
    } // reg drop：RINGS 锁在此释放（不跨到 map_shared / register_task_ring）

    let va = map_shared(space, &meta, bytes)?;

    // open 方自持两端引用（Drop 钩子逐一递减）。
    register_task_ring(meta.id);

    Ok((meta.id, va))
}

/// 加入已有 ring（envcall RingJoin 入口）：a0 = id。
/// 同 team（本方 space 已含该共享块映射）→ 复用既有视图；跨 team → 重新借用
/// 映射同一物理块进本方 space。返回本地视图基址。
///
/// # Errors
/// - `Dead` — id 未登记（对端 ring 不存在）。
pub fn join(space: &Arc<Space>, id: usize) -> Result<usize, RingError> {
    let meta = ring(id).ok_or(RingError::Dead)?;
    let va = map_shared(space, &meta, meta.bytes).map_err(|_| RingError::Dead)?;
    register_task_ring(id);
    Ok(va)
}

/// 终止 ring（envcall RingClose 入口）：置 Dead（对端感知断开）。
pub fn close(id: usize) -> bool {
    let Some(meta) = ring(id) else {
        return false;
    };
    // SAFETY: base 为本 ring 共享区。
    let sh = unsafe { RingShared::new_unchecked(meta.base.as_ptr()) };
    sh.dead();
    true
}

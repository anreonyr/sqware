// Pole — 页级安全内存。
//
// PoleMeta 是内核侧"地基"：物理页块 + 各 pie 的视图登记（键 = token）。
// 用户态 Pie<PoleMeta>（含 Weak<PoleMeta>）只持门闩；map 后用户直接读写页。
//
// 数据面原语：`open` / `shut` / `narrow` / `seal`。创建：`unseal(size)`。
// PoleMeta 拥有物理帧；Arc 归零时 `Drop` 链逐视图 unmap + 还帧。
//
// **`size` 是这一段有多大**：外来区按页界向两侧撑开，故它是页对齐后的长度；
// 设备树 `reg` 声明的那一段（所有权粒度）由调用方自己记。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr::NonNull;

use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{SegmentKind, Space, Span};

use crate::work::unit::gate::GateError;

/// Pole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoleState {
    Live,
    Dead,
}

/// 载荷归属：这块物理区**是谁的**——决定 `Drop` 时做什么。
///
/// 这不是"设备规则"，是**所有权规则**：两条构造路径产出的是同一种门闩，差别只在
/// 这块内存的来路。于是设备（一段有主的、可映射的内存）不需要新名词，也不需要内核
/// 认识"设备"二字。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// 内核自己分配的页块（`frame::allocator()` 给的）：清零、Drop 时归还。
    Frames,
    /// 接管的**外来物理区**（boot 从设备树交出的 MMIO / 保留区）：**不是我的**
    /// ⇒ 不清零、不归还。借映（`open`）与撤映射（`shut`/`Drop`）照旧。
    Region,
}

/// Pole 数据面实体（Arc 持有；最后强引用 drop 时按载荷归还）。
pub struct PoleMeta {
    state: SpinLock<PoleState>,
    /// 载荷归属（构造期定型，无 setter）。
    payload: Payload,
    /// 共享物理块首址（恒等映射下 = PA）。
    base: NonNull<u8>,
    /// 这一段多大（页对齐）。`Open` 把它与 VA 一起返给域——**只有内核知道它**。
    size: usize,
    /// 已映射 (token, Space, Span)。键是 per-pie 身份（全局唯一）而非 per-space：
    /// 同一物理页借映给共享 Space 的多 Task 时，每个 pie 一条独立映射、独立 PTE，
    /// narrow/map 只动自己的那条——cap ⊆ 页表不被共享 PTE 击穿。
    mappings: SpinLock<Vec<(usize, alloc::sync::Weak<Space>, Span)>>,
    /// 开辟者：`UnsealPole` 时的任务 id（构造期定型，无 setter）。0 = 内核自建。
    /// 语义同 `HoleMeta::owner`：`vestor` 管门闩的来历，`owner` 管资源的来历。
    owner: usize,
}

// SAFETY: PoleMeta 经 Arc 跨任务共享；base 指向共享物理帧（仅经 atomic / 直接拷贝
// 访问用户态共享），Send/Sync 安全。
unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    pub(super) fn allocate(size: usize, owner: usize) -> Result<Arc<Self>, GateError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(GateError::NotAligned);
        }
        let layout = core::alloc::Layout::from_size_align(size, PAGE_SIZE)
            .map_err(|_| GateError::NotAligned)?;
        let ptr = crate::tag!(
            Pole,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| GateError::OoM)?
        );
        // SAFETY: 分配返回非空；清零。
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        unsafe {
            core::ptr::write_bytes(base.as_ptr(), 0, size);
        }
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Frames,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
        }))
    }

    /// 接管一段**外来物理区**（boot 从设备树的 `reg` 交出：MMIO 或保留区）。
    ///
    /// 与 [`PoleMeta::allocate`] 的差别只有两处，都在"这块不是我的"这一个事实上：
    /// **不清零**（原样保留外来自述——清零设备寄存器是荒谬的）、**Drop 不归还**
    /// （它不是分配器给的，还回去就是还错东西）。
    ///
    /// `reg` 是设备树声明的那一段（所有权粒度），页是映射粒度：区间按页界向两侧
    /// 撑开，故同一页里的邻居对持有者可见——UART 的 `reg` 只有 0x100，撑到一页。
    /// 存下来的 `size` 因此**大于** `reg`。
    pub(super) fn region(base: usize, reg: usize, owner: usize) -> Result<Arc<Self>, GateError> {
        if reg == 0 {
            return Err(GateError::NotAligned);
        }
        let end = base.checked_add(reg).ok_or(GateError::NotAligned)?;
        let lo = base & !(PAGE_SIZE - 1);
        let hi = end.next_multiple_of(PAGE_SIZE);
        let base = NonNull::new(lo as *mut u8).ok_or(GateError::NotAligned)?;
        let size = hi - lo;
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            payload: Payload::Region,
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            owner,
        }))
    }

    /// 资源开辟者（见字段 `owner`）。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    pub(crate) fn alive(&self) -> bool {
        *self.state.lock() == PoleState::Live
    }

    /// 把物理块借映进 `space`，并登记视图（键 = per-pie token）。
    ///
    /// `flags` 由 caller 算（envcall 入口按 pie subset 决定：FETCH→R，FETCH\|STORE→R\|W），
    /// 本函数不读权限——cap ⊆ 页表的语义靠 caller 守。
    ///
    /// `token` 唯一标识调用方 pie；同 token 复用既有视图（幂等 map），异 token
    /// 各自独立映射（同一物理页可出现在同 space 的多个 VA）。
    fn open_into(
        &self,
        token: usize,
        space: &Arc<Space>,
        flags: PteFlags,
    ) -> Result<usize, GateError> {
        {
            let m = self.mappings.lock();
            if let Some((_, _, span)) = m.iter().find(|(t, _, _)| *t == token) {
                return Ok(span.va.as_usize());
            }
        }
        let va = space
            .with_flush(|inner| {
                let va = inner.allocate(SegmentKind::Normal, self.size)?;
                // 装配失败 ⇒ 只剩段要还（`SpaceInner::allocate` 那条不变量的现场）：
                // `borrow` 在登记之前就拒，maps 干净；此刻一片 PTE 未落，故还段
                // 不必等清退，当场还即可。此前这里用 `?` 直返——**段永久泄漏**，
                // 且被 `GateError::OoM` 掩成"物理内存不够"。
                if let Err(e) = inner.borrow(
                    va,
                    PhysAddr::from_raw(self.base.as_ptr() as usize),
                    self.size,
                    flags,
                ) {
                    inner.deallocate(SegmentKind::Normal, va.as_usize(), self.size);
                    return Err(e);
                }
                Ok::<_, MapError>(va)
            })
            .map_err(|_| GateError::OoM)?;
        // 视图清单：**先备后插**（这一步失败时把刚建好的映射当场撤掉）。
        let mut maps = self.mappings.lock();
        if maps.try_reserve(1).is_err() {
            drop(maps);
            let _ = space.release(Span::new(SegmentKind::Normal, va, self.size, None));
            return Err(GateError::OoM);
        }
        maps.push((
            token,
            Arc::downgrade(space),
            Span::new(SegmentKind::Normal, va, self.size, None),
        ));
        Ok(va.as_usize())
    }

    /// Narrow 降权：把 `token` 对应映射段 `protect` 到新 flags。
    ///
    /// cap ⊆ 页表：narrow 收窄 pie 权限后，该 pie 的映射段 PTE 必须同步降权。
    /// 只动 `token` 自己的映射（未映射则无事）；其他 pie（含同 space 的）不受影响。
    fn narrow_into(&self, token: usize, flags: PteFlags) -> Result<(), GateError> {
        // 锁内只查 + 升级 Arc（锁序纪律：mappings 锁不跨 space 操作）。
        let target = {
            let m = self.mappings.lock();
            m.iter()
                .find(|(t, _, _)| *t == token)
                .and_then(|(_, w, s)| {
                    w.upgrade()
                        .map(|space| (space, s.va.as_usize(), s.size.get()))
                })
        };
        if let Some((space, va, size)) = target {
            space
                .protect(VirtAddr::from_raw(va), size, flags)
                .map_err(|_| GateError::Denied)?;
        }
        Ok(())
    }

    fn shut_from(&self, token: usize) -> Result<(), GateError> {
        let (space, span) = {
            let mut m = self.mappings.lock();
            let pos = m.iter().position(|(t, _, _)| *t == token);
            match pos {
                Some(i) => {
                    let (_, w, s) = m.remove(i);
                    match w.upgrade() {
                        Some(space) => (space, s),
                        None => return Ok(()), // Space 已死，映射随 Space drop 消失
                    }
                }
                None => return Ok(()), // 幂等
            }
        };
        space.release(span).map_err(|_| GateError::Denied)
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = PoleState::Dead;
        let mappings: Vec<(usize, alloc::sync::Weak<Space>, Span)> =
            core::mem::take(&mut *self.mappings.lock());
        for (_, weak, span) in mappings {
            if let Some(space) = weak.upgrade() {
                let _ = space.release(span);
            }
        }
        let layout =
            core::alloc::Layout::from_size_align(self.size, PAGE_SIZE).expect("pole layout valid");
        // 载荷归属决定这一句：**分配器给的才还回去**。外来区（`Region`）在此什么都不做
        // ——它不是内核的内存，还它就是还错东西。
        if self.payload == Payload::Frames {
            unsafe {
                frame::allocator().deallocate(self.base, layout);
            }
        }
    }
}

// ── 数据面原语 ──

/// 把物理页借映进 `space`（需 rights & R，flags 由 caller 按 subset 决定）。
/// `token` = 调用方 pie 的映射身份；同 token 幂等复用，异 token 独立映射。
///
/// 返 `(VA, 这一段多大)`——**两件一起返**：长度只在内核手里（外来区按页界撑开，
/// 设备树 `reg` 的长度内核不知道），分开取就会把它留成调用方的猜测。
pub(crate) fn open(
    meta: &PoleMeta,
    token: usize,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<(usize, usize), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let va = meta.open_into(token, space, flags)?;
    // 强制翻 PTE flags——map_into 偶遇 superpage / 旧 entry 时 flags 没真落位；
    // protect 走 walk 改 PTE flags，确保 cap ⊆ 页表（无视 superpage 起点）。
    let _ = space.protect(VirtAddr::from_raw(va), meta.size, flags);
    Ok((va, meta.size))
}

/// 从 `space` 解除映射（幂等）。`token` 定位该 pie 的映射。
///
/// **不过存活闸**：撤的是**我自己那张 PTE**，与资源活不活着无关——`Seal` 之后
/// 仍得能撤。这与 `Release` 是同一条语义：「你总得能放下手里的东西」（见 `fid.rs`
/// 的 `Release` 那一格）。`shut_from` 本就幂等：未映射、Space 已死都返 `Ok`。
pub(crate) fn shut(meta: &PoleMeta, token: usize) -> Result<(), GateError> {
    meta.shut_from(token)
}

/// Narrow 降权：把 `token` 对应映射段降权到新 `flags`（cap ⊆ 页表）。未映射则无事。
pub(crate) fn narrow(meta: &PoleMeta, token: usize, flags: PteFlags) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    meta.narrow_into(token, flags)
}

/// 封印 Pole（置死；物理帧与映射随**最后一份强引用** drop 归还）。
///
/// 不回收内存——资源寿命由引用计数决定。调用方不持 L3 锁。
pub(crate) fn seal(meta: &PoleMeta) {
    *meta.state.lock() = PoleState::Dead;
}

// ── 创建 ──

/// 解封 Pole 的资源实体：分配物理页 + 建 Meta。**不落 pies、不 auto-map**——
/// 建门闩 + 落 `task.pies` + map 创建者视图由 envcall 编排（gate::new_pie +
/// pies.push + pole::map）。返 `Arc`：它既是资源实体，也是门闩持有的**唯一强
/// 引用**（资源寿命 = 能力寿命；最后一份消失时 `Drop` 归还帧 + 撤映射）。
/// `owner` = 开辟者任务 id（envcall 入口传当前任务）。
pub(crate) fn meta(size: usize, owner: usize) -> Result<Arc<PoleMeta>, GateError> {
    PoleMeta::allocate(size, owner)
}

/// 接管一段外来物理区，做成一枚门闩的资源实体（见 [`PoleMeta::region`]）。
///
/// **只对内核开放**（`pub(crate)`，无 envcall 入口）：设备树是 boot 的事实，
/// 域不能凭一个物理地址给自己造门闩。独占因此不靠判据，靠**没有第二个创建入口**。
pub(crate) fn region(base: usize, reg: usize, owner: usize) -> Result<Arc<PoleMeta>, GateError> {
    PoleMeta::region(base, reg, owner)
}

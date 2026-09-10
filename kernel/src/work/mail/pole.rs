// Pole — 页级安全内存。
//
// PoleMeta 是内核侧"地基"：物理页块 + 各 pie 的视图登记（键 = token）。
// 用户态 Pie<PoleMeta>（含 Weak<PoleMeta>）只持门闩；map 后用户直接读写页。
//
// 数据面原语：`open` / `shut` / `narrow` / `seal`。创建：`unseal(bytes)`。
// PoleMeta 拥有物理帧；Arc 归零时 `Drop` 链逐视图 unmap + 还帧。

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ptr::NonNull;

use crate::lock::{Level, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{Seg, Space, Span};

use crate::work::unit::gate::GateError;

/// Pole 状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoleState {
    Live,
    Dead,
}

/// Pole 数据面实体（Arc 持有；最后强引用 drop 时物理帧归还）。
pub struct PoleMeta {
    state: SpinLock<PoleState>,
    /// 共享物理块首址（恒等映射下 = PA）。
    base: NonNull<u8>,
    /// 字节数（页对齐）。
    bytes: usize,
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
    pub(super) fn allocate(bytes: usize, owner: usize) -> Result<Arc<Self>, GateError> {
        if bytes == 0 || !bytes.is_multiple_of(PAGE_SIZE) {
            return Err(GateError::NotAligned);
        }
        let layout = core::alloc::Layout::from_size_align(bytes, PAGE_SIZE)
            .map_err(|_| GateError::NotAligned)?;
        let ptr = crate::tag!(
            Ring,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| GateError::OoM)?
        );
        // SAFETY: 分配返回非空；清零。
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        unsafe {
            core::ptr::write_bytes(base.as_ptr(), 0, bytes);
        }
        Ok(Arc::new(Self {
            state: SpinLock::new_level(Level::L3, PoleState::Live),
            base,
            bytes,
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
    /// `flags` 由 caller 算（envcall 入口按 pie subset 决定：READ→R，READ\|WRITE→R\|W），
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
                let va = inner.allocate(Seg::User, self.bytes)?;
                inner.borrow(
                    va,
                    PhysAddr::from_raw(self.base.as_ptr() as usize),
                    self.bytes,
                    flags,
                )?;
                Ok::<_, MapError>(va)
            })
            .map_err(|_| GateError::OoM)?;
        self.mappings.lock().push((
            token,
            Arc::downgrade(space),
            Span::new(Seg::User, va, self.bytes, None),
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
        if let Some((space, va, bytes)) = target {
            space
                .protect(VirtAddr::from_raw(va), bytes, flags)
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
            core::alloc::Layout::from_size_align(self.bytes, PAGE_SIZE).expect("pole layout valid");
        unsafe {
            frame::allocator().deallocate(self.base, layout);
        }
    }
}

// ── 数据面原语 ──

/// 把物理页借映进 `space`（需 rights & R，flags 由 caller 按 subset 决定）。
/// `token` = 调用方 pie 的映射身份；同 token 幂等复用，异 token 独立映射。
pub(crate) fn open(
    meta: &PoleMeta,
    token: usize,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<usize, GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
    let va = meta.open_into(token, space, flags)?;
    // 强制翻 PTE flags——map_into 偶遇 superpage / 旧 entry 时 flags 没真落位；
    // protect 走 walk 改 PTE flags，确保 cap ⊆ 页表（无视 superpage 起点）。
    let _ = space.protect(VirtAddr::from_raw(va), meta.bytes, flags);
    Ok(va)
}

/// 从 `space` 解除映射（幂等；需 rights & (R | W)）。`token` 定位该 pie 的映射。
pub(crate) fn shut(meta: &PoleMeta, token: usize) -> Result<(), GateError> {
    if !meta.alive() {
        return Err(GateError::Dead);
    }
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
pub(crate) fn meta(bytes: usize, owner: usize) -> Result<Arc<PoleMeta>, GateError> {
    PoleMeta::allocate(bytes, owner)
}

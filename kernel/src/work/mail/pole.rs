// Pole — 页级安全内存。
//
// PoleMeta 是内核侧"地基"：物理页块 + 各 Space 的视图登记。
// 用户态 Pie<Pole>（含 Weak<PoleMeta>）只持门闩；map 后用户直接读写页。
//
// 数据面原语：`pole_map` / `pole_unmap` / `pole_shut`。
// 创建：`pole_create(space, bytes)` —— 分配物理页 + 建 Meta + 注册 + auto-map 当前 Space。
//
// PoleMeta 拥有物理帧；Arc 归零时由 `Drop` 链逐视图 unmap + 还帧。

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

use super::pie::{MailError, Permission};
use super::resource_table::{self, ResourceId};

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
    /// 已映射 (pie_token, Space, Span)。
    ///
    /// 键是 **per-pie 身份**（`Pie::token`，全局唯一）而非 per-space：同一物理页
    /// 借映给同一 Team 里多个 Task（共享 Space）时，每个 pie 一条独立映射、独立
    /// PTE——restrict/map 只动自己的那条，保证 cap ⊆ 页表不被共享 PTE 击穿。
    mappings: SpinLock<Vec<(u64, alloc::sync::Weak<Space>, Span)>>,
}

// SAFETY: PoleMeta 经 Arc 跨任务共享；base 指向共享物理帧（仅经 atomic / 直接拷贝
// 访问用户态共享），Send/Sync 安全。
unsafe impl Send for PoleMeta {}
unsafe impl Sync for PoleMeta {}

impl PoleMeta {
    pub(super) fn allocate(bytes: usize) -> Result<Arc<Self>, MailError> {
        if bytes == 0 || !bytes.is_multiple_of(PAGE_SIZE) {
            return Err(MailError::NotAligned);
        }
        let layout = core::alloc::Layout::from_size_align(bytes, PAGE_SIZE)
            .map_err(|_| MailError::NotAligned)?;
        let ptr = crate::tag!(
            Task,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| MailError::OOM)?
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
        }))
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
    fn map_into(&self, token: u64, space: &Arc<Space>, flags: PteFlags) -> Result<usize, MailError> {
        {
            let m = self.mappings.lock();
            if let Some((_, _, span)) = m
                .iter()
                .find(|(t, _, _)| *t == token)
            {
                return Ok(span.va.as_usize());
            }
        }
        let va = space
            .with_flush(|inner| {
                let va = inner.allocate(Seg::User, self.bytes)?;
                inner.borrow_map(
                    va,
                    PhysAddr::from_raw(self.base.as_ptr() as usize),
                    self.bytes,
                    flags,
                )?;
                Ok::<_, MapError>(va)
            })
            .map_err(|_| MailError::OOM)?;
        self.mappings
            .lock()
            .push((token, Arc::downgrade(space), Span::new(Seg::User, va, self.bytes, None)));
        Ok(va.as_usize())
    }

    /// Restrict 降权：把 `token` 对应映射段 `protect` 到新 flags。
    ///
    /// cap ⊆ 页表：restrict 收窄 pie 权限后，该 pie 的映射段 PTE 必须同步降权。
    /// 只动 `token` 自己的映射（未映射则无事）；其他 pie（含同 space 的）不受影响。
    fn restrict_into(&self, token: u64, flags: PteFlags) -> Result<(), MailError> {
        // 锁内只查 + 升级 Arc（锁序纪律：mappings 锁不跨 space 操作）。
        let target = {
            let m = self.mappings.lock();
            m.iter()
                .find(|(t, _, _)| *t == token)
                .and_then(|(_, w, s)| w.upgrade().map(|space| (space, s.va.as_usize(), s.size.get())))
        };
        if let Some((space, va, bytes)) = target {
            space
                .protect(VirtAddr::from_raw(va), bytes, flags)
                .map_err(|_| MailError::Denied)?;
        }
        Ok(())
    }

    fn unmap_from(&self, token: u64) -> Result<(), MailError> {
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
        space.release(span).map_err(|_| MailError::Denied)
    }
}

impl Drop for PoleMeta {
    fn drop(&mut self) {
        *self.state.lock() = PoleState::Dead;
        let mappings: Vec<(u64, alloc::sync::Weak<Space>, Span)> =
            core::mem::take(&mut *self.mappings.lock());
        for (_, weak, span) in mappings {
            if let Some(space) = weak.upgrade() {
                let _ = space.release(span);
            }
        }
        let layout = core::alloc::Layout::from_size_align(self.bytes, PAGE_SIZE)
            .expect("pole layout valid");
        unsafe {
            frame::allocator().deallocate(self.base, layout);
        }
    }
}

// ── 数据面原语 ──

/// 把物理页借映进 `space`（需 rights & R，flags 由 caller 按 subset 决定）。
/// `token` = 调用方 pie 的映射身份；同 token 幂等复用，异 token 独立映射。
pub(crate) fn pole_map(
    meta: &PoleMeta,
    token: u64,
    space: &Arc<Space>,
    flags: PteFlags,
) -> Result<usize, MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    let va = meta.map_into(token, space, flags)?;
    // 强制翻 PTE flags——map_into 偶遇 superpage / 旧 entry 时 flags 没真落位；
    // protect 走 walk 改 PTE flags，确保 cap ⊆ 页表（无视 superpage 起点）。
    let _ = space.protect(
        crate::memory::manager::addr::VirtAddr::from_raw(va),
        meta.bytes,
        flags,
    );
    Ok(va)
}

/// 从 `space` 解除映射（幂等；需 rights & (R | W)）。`token` 定位该 pie 的映射。
pub(crate) fn pole_unmap(meta: &PoleMeta, token: u64) -> Result<(), MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    meta.unmap_from(token)
}

/// Restrict 降权：把 `token` 对应映射段降权到新 `flags`（cap ⊆ 页表）。未映射则无事。
pub(crate) fn pole_restrict(meta: &PoleMeta, token: u64, flags: PteFlags) -> Result<(), MailError> {
    if !meta.alive() {
        return Err(MailError::Dead);
    }
    meta.restrict_into(token, flags)
}

/// 终止 Pole（state = Dead + 资源表移除；Arc drop 时归还物理帧）。
pub(crate) fn pole_shut(meta: &PoleMeta, id: ResourceId) {
    *meta.state.lock() = PoleState::Dead;
    resource_table::remove(id, super::pie::PieKind::Pole);
}

// ── 创建 ──

use crate::work::room::scheduler::core::current;

/// 创建 Pole：分配物理页 + 建 Meta + 注册 + auto-map 进当前 task 所在 Space +
/// 推 AnyPie::Pole 到 task.pies。
pub(crate) fn pole_create(space: &Arc<Space>, bytes: usize) -> Result<usize, MailError> {
    let arc = PoleMeta::allocate(bytes)?;
    let id = resource_table::alloc_id();
    resource_table::insert_pole(id, &arc);

    let task = current().running_task().ok_or(MailError::Denied)?;
    let task_space = task.ident.team.space.clone();
    // 先造 pie（拿到 per-pie token），再用该 token auto-map 创建者的视图——
    // 创建者 pie 的映射独立成条，后续 restrict 只降它自己的 PTE。
    let pie = super::pie::new_pie::<super::pie::Pole>(
        id,
        Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
        None, // 原始创建者：无 vestor
        alloc::sync::Arc::downgrade(&arc),
    );
    // 创建者自留 pie 全权（R|W|VEST）→ map 走 R|W。
    let creator_flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
    arc.map_into(pie.token, &task_space, creator_flags)?;

    let mut pies = task.pies.lock();
    pies.push(super::pie::AnyPie::Pole(pie));
    let _ = space; // suppress unused; auto-map 用 task.ident.team.space
    Ok(pies.len() - 1)
}
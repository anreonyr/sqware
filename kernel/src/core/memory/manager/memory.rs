use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;

use log::{error, info};
use riscv::register::satp;

use super::asid::Asid;
use super::fault::FaultKind;
use super::frame::Frame;
use super::kind::SpaceKind;
use super::{access::Access, mode, tree};
use crate::core::memory::PAGE_SIZE;
use crate::core::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::MapError;
use crate::machine::layout::{TEAM_FRAME_BASE, TEAM_FRAME_WINDOW_SIZE};
use crate::machine::sync::{Level, SpinLock};

mod cleanup;
mod mapping;
mod range_reclaim;
mod reclaim;
mod reservation;
mod va_allocator;

pub use mapping::Mapping;
pub use reservation::Reservation;
pub(crate) use va_allocator::RegionId;

use cleanup::CleanupNode;
use range_reclaim::RangeReclaim;
use reclaim::Reclaim;
use va_allocator::VaAllocator;

#[cfg(debug_assertions)]
static FAIL_CLEANUP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(debug_assertions)]
pub(crate) fn fail_cleanup_prepare() {
    assert!(crate::testing());
    FAIL_CLEANUP.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// 地址空间内的分配、映射与待完成回收。
pub struct Memory {
    tree: Arc<tree::Tree>,
    allocator: SpinLock<VaAllocator>,
    pending_cleanup: SpinLock<Option<Box<CleanupNode>>>,
    kind: SpaceKind,
}

impl Memory {
    pub(crate) fn new(kind: SpaceKind, kernel: bool) -> Result<Self, MapError> {
        let tree = if kernel {
            tree::Tree::new_kernel()?
        } else {
            tree::Tree::new()?
        };
        let mut allocator = VaAllocator::new();
        allocator.attach(
            RegionId::Normal,
            crate::machine::layout::IMAGE_BASE.as_usize(),
            mode::upper().as_usize(),
        )?;
        allocator.attach(
            RegionId::Kernel,
            TEAM_FRAME_BASE.as_usize(),
            TEAM_FRAME_BASE.as_usize() + TEAM_FRAME_WINDOW_SIZE,
        )?;
        if kernel {
            let dram = crate::machine::board::info().dram;
            let base = mode::lower()
                .as_usize()
                .checked_add(dram.base)
                .and_then(|base| base.checked_add(dram.size))
                .ok_or(MapError::DramOverlap)?;
            allocator.attach(
                RegionId::Resource,
                base,
                crate::machine::layout::TRAP_STACK_BASE.as_usize(),
            )?;
        }
        Ok(Self {
            tree,
            allocator: SpinLock::new_level(Level::Space, allocator),
            pending_cleanup: SpinLock::new_level(Level::Space, None),
            kind,
        })
    }

    pub fn kind(&self) -> SpaceKind {
        self.kind
    }

    /// 白盒借用：裸页表，只给健康用例与调试核对直查，普通路径不得使用。
    #[cfg(debug_assertions)]
    pub(crate) fn backend(&self) -> &Arc<tree::Tree> {
        &self.tree
    }

    pub(crate) fn asid(&self) -> Asid {
        self.tree.asid()
    }

    pub(crate) fn satp(&self) -> satp::Satp {
        self.tree.satp()
    }

    pub(crate) fn root_ppn(&self) -> usize {
        self.tree.root_ppn()
    }

    /// 翻译里只取物理地址的那一面：权限位不外露。
    pub(crate) fn physical_at(&self, address: VirtAddr) -> Option<PhysAddr> {
        self.tree.translate(address).map(|(physical, _)| physical)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        self.tree.audit();
    }

    /// 启动期静态映射：页表里直接落记录，不占地址桶的保留格。
    pub(crate) fn map_boot_frames(
        &self,
        start: VirtAddr,
        frames: Vec<Arc<Frame>>,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<(), MapError> {
        self.tree.map_frames(start, frames, ceiling, current)?;
        Ok(())
    }

    /// 映射一段调用方仍保留的物理区间。
    pub(crate) fn map_boot_borrowed(
        &self,
        start: VirtAddr,
        region: Arc<tree::BorrowedRegion>,
        offset: usize,
        size: usize,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<(), MapError> {
        self.tree.map_borrowed(start, region, offset, size, ceiling, current)?;
        Ok(())
    }

    /// 把一条已有映射的一段权限收窄到 `ceiling`。
    ///
    /// 身份由 `source` 自己带，所有权核在树里。
    pub(crate) fn narrow_record(
        &self,
        source: &Arc<tree::Map>,
        start: VirtAddr,
        size: usize,
        ceiling: PteFlags,
    ) -> Result<(), MapError> {
        self.tree.narrow(source, start, size, ceiling)
    }

    /// 把 kernel 团队的跳板映射复制进来；来源由调用方给出。
    pub(crate) fn seed_trampoline(&self, source: &Memory) -> Result<(), MapError> {
        let (physical, flags) = source
            .tree
            .translate(crate::machine::layout::TRAMPOLINE)
            .ok_or(MapError::NoRegion)?;
        let region = unsafe { tree::BorrowedRegion::new(physical, PAGE_SIZE)? };
        self.tree.map_borrowed(
            crate::machine::layout::TRAMPOLINE,
            region,
            0,
            PAGE_SIZE,
            flags,
            flags,
        )?;
        Ok(())
    }

    pub(crate) fn pte_policy(&self, flags: PteFlags) -> PteFlags {
        if self.kind.is_supervisor() {
            flags - PteFlags::U
        } else {
            flags | PteFlags::U
        }
    }

    fn mapping_policy(&self, region: RegionId, flags: PteFlags) -> PteFlags {
        if region != RegionId::Normal {
            flags - PteFlags::U
        } else {
            self.pte_policy(flags)
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn attach_region(
        &self,
        region: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), MapError> {
        self.allocator.lock().configure(region, base, edge)
    }

    pub(crate) fn advance_region(&self, region: RegionId, base: usize) -> Result<(), MapError> {
        self.allocator.lock().advance(region, base)
    }

    pub(crate) fn mapping_at(&self, start: VirtAddr, size: usize) -> Result<Mapping, MapError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let map = self.tree.find(start).ok_or(MapError::NoRegion)?;
        let (map_start, map_size) = map.initial_range();
        if map_start != start || map_size != size || !map.is_live() {
            return Err(MapError::NoRegion);
        }
        let region = self
            .allocator
            .lock()
            .exact_reservation(start.as_usize(), size)
            .ok_or(MapError::NoRegion)?;
        Ok(Mapping {
            map,
            reservation: Reservation {
                tree: Arc::downgrade(&self.tree),
                region: region.0,
                start,
                size,
                id: region.1,
            },
        })
    }

    /// 普通路径的按字节申请：eager 物化、`Normal` 桶、返回稳定收据。
    ///
    /// `layout` 的尺寸折成实际信封（至少 1 字节，再按页 granule 取整），对齐取
    /// `max(layout.align(), PAGE_SIZE)`。`at` 给定时必须同时满足两者，否则
    /// `NotAligned`；自动选址时取满足对齐的最低空档，不为对齐另开大页机制。
    /// 新建存储的上限是 `Access::all()`，初始 current 取 `access` 加 A/D。
    #[allow(dead_code)]
    pub fn allocate(
        &self,
        layout: Layout,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.allocate_eager(Self::envelope(layout)?, Self::alignment(layout), access, at)
    }

    /// ABI `Allocate` 的字节入口：尺寸按 basic 口径定点。
    pub(crate) fn allocate_bytes(
        &self,
        size: usize,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.allocate_eager(Self::basic_bytes(size)?, PAGE_SIZE, access, at)
    }

    fn allocate_eager(
        &self,
        size: usize,
        align: usize,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(size / PAGE_SIZE)
            .map_err(|_| env::MemoryFail::OoM)?;
        for _ in 0..size / PAGE_SIZE {
            frames.push(Frame::allocate()?);
        }
        self.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        let current = access.to_pte() | PteFlags::A | PteFlags::D;
        self.map_frames(reservation, frames, Access::all().to_pte(), current)
            .map_err(Into::into)
    }

    /// 普通路径的惰性映射：源是 `None` 的那条 `Mmap`。
    ///
    /// `limit` 是授权上限，`current` 是初始逻辑权限；A/D 由 `Memory` 补上，调用方不写。
    /// 选址对齐取 `max(layout.align(), PAGE_SIZE)`。
    #[allow(dead_code)]
    pub fn allocate_lazy(
        &self,
        layout: Layout,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.allocate_lazy_core(
            Self::envelope(layout)?,
            Self::alignment(layout),
            limit,
            current,
            at,
        )
    }

    /// ABI `Mmap` 无来源那条：线上尺寸已按页定点，选址对齐取 `PAGE_SIZE`。
    pub(crate) fn allocate_lazy_bytes(
        &self,
        size: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.allocate_lazy_core(size, PAGE_SIZE, limit, current, at)
    }

    fn allocate_lazy_core(
        &self,
        size: usize,
        align: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        let current = current.to_pte() | PteFlags::A | PteFlags::D;
        self.map_lazy(reservation, limit.to_pte(), current)
            .map_err(Into::into)
    }

    /// 普通路径的派生映射：从 `source` 收据的第 `offset` 字节起，取 `layout` 那么
    /// 大的一段。源已物化的帧共享；源 lazy 时先按原策略物化。
    #[allow(dead_code)]
    pub fn view(
        &self,
        source: &Mapping,
        offset: usize,
        layout: Layout,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.view_core(
            &source.map,
            offset,
            Self::envelope(layout)?,
            Self::alignment(layout),
            limit,
            current,
            at,
        )
    }

    /// `Arc<Map>` 入口：`offset` 与 `size` 已按页定点。
    pub(crate) fn view_record(
        &self,
        source: &Arc<tree::Map>,
        offset: usize,
        size: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        self.view_core(source, offset, size, PAGE_SIZE, limit, current, at)
    }

    fn view_core(
        &self,
        source: &Arc<tree::Map>,
        offset: usize,
        size: usize,
        align: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, env::MemoryFail> {
        Self::frame_offset(offset)?;
        self.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        let current = current.to_pte() | PteFlags::A | PteFlags::D;
        self.derive(reservation, source, offset, limit.to_pte(), current)
            .map_err(Into::into)
    }

    /// 一次保护一个页对齐区间：先整体验完覆盖与上限，再按原次序逐段 protect。
    /// 成功带 EXECUTE 时在末尾做一次指令同步。
    pub fn protect_range(
        &self,
        addr: VirtAddr,
        size: usize,
        access: Access,
    ) -> Result<(), env::MemoryFail> {
        Self::fixed_range(addr, size)?;
        self.retry_invalidation()?;
        let end = addr.as_usize() + size;
        let flags = access.to_pte();
        let ranges = self.tree.ranges()?;
        let mut updates = Vec::new();
        updates
            .try_reserve(ranges.len())
            .map_err(|_| env::MemoryFail::OoM)?;
        let mut covered = addr.as_usize();
        for (map, range) in ranges {
            if range.start.as_usize() >= end || range.end() < addr.as_usize() {
                continue;
            }
            let lo = range.start.as_usize().max(addr.as_usize());
            let hi = (range.end() + 1).min(end);
            if lo != covered {
                return Err(env::MemoryFail::NoRegion);
            }
            if !range.ceiling.contains(flags) {
                return Err(env::MemoryFail::WidenDenied);
            }
            updates.push((map, VirtAddr::wrap(lo), hi - lo));
            covered = hi;
        }
        if covered != end {
            return Err(env::MemoryFail::NoRegion);
        }
        for (map, start, size) in updates {
            self.tree.protect(&map, start, size, flags)?;
        }
        if access.contains(Access::EXECUTE) {
            super::asid::sync_instructions()?;
        }
        Ok(())
    }

    /// 与 `[start, start + size)` 相交的稳定映射记录，每个 `Map` 只出现一次。
    ///
    /// 只交出身份，不交出区间与页表状态。
    pub(crate) fn mappings_in(
        &self,
        start: VirtAddr,
        size: usize,
    ) -> Result<Vec<Arc<tree::Map>>, env::MemoryFail> {
        let ranges = self.tree.ranges()?;
        let end = start.as_usize().saturating_add(size);
        let mut maps = Vec::new();
        maps.try_reserve(ranges.len())
            .map_err(|_| env::MemoryFail::OoM)?;
        for (map, range) in ranges {
            if range.start.as_usize() < end
                && range.end() >= start.as_usize()
                && !maps.iter().any(|held| Arc::ptr_eq(held, &map))
            {
                maps.push(map);
            }
        }
        Ok(maps)
    }

    /// basic 口径的线上尺寸：最少 1 字节，向上取整到页；溢出仍是 `Denied`。
    pub(crate) fn basic_bytes(raw: usize) -> Result<usize, env::MemoryFail> {
        raw.max(1)
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(env::MemoryFail::Denied)
    }

    /// 非 basic 口径的线上尺寸：非零且按页对齐，否则 `NotAligned`。
    pub(crate) fn exact_bytes(raw: usize) -> Result<usize, env::MemoryFail> {
        if raw == 0 || !raw.is_multiple_of(PAGE_SIZE) {
            return Err(env::MemoryFail::NotAligned);
        }
        Ok(raw)
    }

    /// 定点区间：`addr` 必须是页地址，且整段落在用户可达范围内。
    pub(crate) fn fixed_range(addr: VirtAddr, size: usize) -> Result<(), env::MemoryFail> {
        if addr.offset() != 0 || !Self::user_range(addr.as_usize(), size) {
            return Err(env::MemoryFail::NotAligned);
        }
        Ok(())
    }

    /// 线上偏移：必须按页对齐，不得让调用方跨页边界泄漏源页前缀。
    pub(crate) fn frame_offset(offset: usize) -> Result<(), env::MemoryFail> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return Err(env::MemoryFail::NotAligned);
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn envelope(layout: Layout) -> Result<usize, env::MemoryFail> {
        Self::basic_bytes(layout.size())
    }

    #[allow(dead_code)]
    fn alignment(layout: Layout) -> usize {
        layout.align().max(PAGE_SIZE)
    }

    /// 选址：`at` 为空时取满足 `align` 的最低空档，否则要它自己也满足 `align`。
    fn place(
        &self,
        size: usize,
        align: usize,
        at: Option<VirtAddr>,
    ) -> Result<Reservation, MapError> {
        let Some(addr) = at else {
            let (start, id) =
                self.allocator
                    .lock()
                    .allocate_aligned_low(RegionId::Normal, size, align)?;
            return Ok(Reservation {
                tree: Arc::downgrade(&self.tree),
                region: RegionId::Normal,
                start: VirtAddr::from_raw(start),
                size,
                id,
            });
        };
        if addr.as_usize() % align != 0 {
            return Err(MapError::NotAligned);
        }
        self.reserve(RegionId::Normal, addr, size)
    }

    pub(crate) fn reserve(
        &self,
        region: RegionId,
        start: VirtAddr,
        size: usize,
    ) -> Result<Reservation, MapError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let id = self
            .allocator
            .lock()
            .reserve(region, start.as_usize(), size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start,
            size,
            id,
        })
    }

    pub(crate) fn allocate_low(&self, region: RegionId, size: usize) -> Result<Reservation, MapError> {
        let (start, id) = self.allocator.lock().allocate_low(region, size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start: VirtAddr::from_raw(start),
            size,
            id,
        })
    }

    pub(crate) fn allocate_high(&self, region: RegionId, size: usize) -> Result<Reservation, MapError> {
        let (start, id) = self.allocator.lock().allocate_high(region, size)?;
        Ok(Reservation {
            tree: Arc::downgrade(&self.tree),
            region,
            start: VirtAddr::from_raw(start),
            size,
            id,
        })
    }

    pub fn split_reservation(
        &self,
        reservation: Reservation,
        left_size: usize,
    ) -> Result<(Reservation, Reservation), MapError> {
        self.check_reservation(&reservation)?;
        let split = self.allocator.lock().split(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
            left_size,
        );
        let (right_start, left_id, right_id) = match split {
            Ok(split) => split,
            Err(error) => {
                let _ = self.give_back(&reservation);
                return Err(error);
            }
        };
        let owner = Arc::downgrade(&self.tree);
        Ok((
            Reservation {
                tree: owner.clone(),
                region: reservation.region,
                start: reservation.start,
                size: left_size,
                id: left_id,
            },
            Reservation {
                tree: owner,
                region: reservation.region,
                start: VirtAddr::from_raw(right_start),
                size: reservation.size - left_size,
                id: right_id,
            },
        ))
    }

    fn give_back(&self, reservation: &Reservation) -> bool {
        self.allocator.lock().release(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
        )
    }

    pub fn release_reservation(&self, reservation: Reservation) -> Result<(), MapError> {
        self.check_reservation(&reservation)?;
        if self.tree.overlaps(reservation.start, reservation.size)? {
            return Err(MapError::AlreadyMapped);
        }
        if self.give_back(&reservation) {
            Ok(())
        } else {
            Err(MapError::NoRegion)
        }
    }

    fn enqueue_cleanup(&self, mut node: Box<CleanupNode>) {
        let mut pending = self.pending_cleanup.lock();
        node.next = pending.take();
        *pending = Some(node);
    }

    fn take_cleanup(&self) -> Option<Box<CleanupNode>> {
        let mut pending = self.pending_cleanup.lock();
        let mut node = pending.take()?;
        *pending = node.next.take();
        Some(node)
    }

    fn prepare_mapping_cleanup(&self, reservation: &Reservation) -> Result<(), MapError> {
        #[cfg(debug_assertions)]
        if FAIL_CLEANUP.swap(false, core::sync::atomic::Ordering::Relaxed) {
            return Err(MapError::OutOfMemory);
        }
        self.allocator.lock().prepare_cleanup(reservation.id)
    }

    fn cancel_mapping_cleanup(&self, reservation: &Reservation) {
        let _ = self.allocator.lock().take_cleanup(reservation.id);
    }

    fn record_map_failure(
        &self,
        reservation: Reservation,
        error: MapError,
    ) -> Result<Mapping, MapError> {
        let map = self.tree.probe(reservation.start).filter(|map| {
            let (start, size) = map.initial_range();
            start == reservation.start
                && size == reservation.size
                && map
                    .tree()
                    .is_some_and(|tree| Arc::ptr_eq(&tree, &self.tree))
        });
        if let Some(map) = map {
            let node = self.allocator.lock().take_cleanup(reservation.id);
            if let Some(mut node) = node {
                node.record = Some(Reclaim::Mapping(Mapping { map, reservation }));
                self.enqueue_cleanup(node);
            }
        } else {
            self.cancel_mapping_cleanup(&reservation);
            let _ = self.give_back(&reservation);
        }
        Err(error)
    }

    fn ensure_unmapped_reservation(&self, reservation: &Reservation) -> Result<(), MapError> {
        if self.tree.overlaps(reservation.start, reservation.size)? {
            let _ = self.give_back(&reservation);
            return Err(MapError::AlreadyMapped);
        }
        Ok(())
    }

    pub(crate) fn map_frames(
        &self,
        reservation: Reservation,
        frames: Vec<Arc<Frame>>,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        if frames.len().checked_mul(PAGE_SIZE) != Some(reservation.size) {
            let _ = self.release_reservation(reservation);
            return Err(MapError::NotAligned);
        }
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.give_back(&reservation);
            return Err(error);
        }
        match self.tree.map_frames(
            reservation.start,
            frames,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub(crate) fn map_lazy(
        &self,
        reservation: Reservation,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.give_back(&reservation);
            return Err(error);
        }
        match self.tree.map_lazy(
            reservation.start,
            reservation.size,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub(crate) unsafe fn map_borrowed(
        &self,
        reservation: Reservation,
        physical: PhysAddr,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        let region = match unsafe { tree::BorrowedRegion::new(physical, reservation.size) } {
            Ok(region) => region,
            Err(error) => {
                let _ = self.release_reservation(reservation);
                return Err(error);
            }
        };
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.give_back(&reservation);
            return Err(error);
        }
        match self.tree.map_borrowed(
            reservation.start,
            region,
            0,
            reservation.size,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub(crate) fn derive(
        &self,
        reservation: Reservation,
        source: &Arc<tree::Map>,
        source_offset: usize,
        ceiling: PteFlags,
        current: PteFlags,
    ) -> Result<Mapping, MapError> {
        self.check_reservation(&reservation)?;
        self.ensure_unmapped_reservation(&reservation)?;
        if let Err(error) = self.prepare_mapping_cleanup(&reservation) {
            let _ = self.give_back(&reservation);
            return Err(error);
        }
        match self.tree.derive_map(
            source,
            source_offset,
            reservation.size,
            reservation.start,
            ceiling,
            self.mapping_policy(reservation.region, current),
        ) {
            Ok(map) => Ok(Mapping { map, reservation }),
            Err(error) => self.record_map_failure(reservation, error),
        }
    }

    pub fn user_range(addr: usize, size: usize) -> bool {
        let upper = mode::upper().as_usize();
        addr < upper && addr.checked_add(size).is_some_and(|end| end <= upper)
    }

    pub fn retry_invalidation(&self) -> Result<(), MapError> {
        self.tree.retry_invalidation()?;
        while let Some(mut node) = self.take_cleanup() {
            let result = match node.record.as_mut().expect("cleanup record") {
                Reclaim::Mapping(mapping) => self.unmap(mapping),
                Reclaim::Range(range) => self.finish_range(range),
            };
            if let Err(error) = result {
                self.enqueue_cleanup(node);
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn unmap_range(&self, start: VirtAddr, size: usize) -> Result<(), MapError> {
        if !Self::user_range(start.as_usize(), size)
            || start.offset() != 0
            || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(MapError::NotAligned);
        }
        self.retry_invalidation()?;
        let end = start.as_usize() + size;
        let ranges = self.tree.ranges()?;
        let mut maps = Vec::new();
        maps.try_reserve(ranges.len())
            .map_err(|_| MapError::OutOfMemory)?;
        for (map, range) in ranges {
            let lo = start.as_usize().max(range.start.as_usize());
            if range.start.as_usize() >= end || range.end() < start.as_usize() {
                continue;
            }
            let hi = (range.end() + 1).min(end);
            if lo < hi {
                maps.push((map, VirtAddr::wrap(lo), hi - lo));
            }
        }
        if maps.is_empty() {
            return Err(MapError::NoRegion);
        }
        let mut cuts = Vec::new();
        for (_, at, len) in &maps {
            let part = self
                .allocator
                .lock()
                .cuts(RegionId::Normal, at.as_usize(), *len)?;
            if part.iter().map(|(_, len, _)| len).sum::<usize>() != *len {
                return Err(MapError::NoRegion);
            }
            cuts.try_reserve(part.len())
                .map_err(|_| MapError::OutOfMemory)?;
            cuts.extend(part);
        }
        let mut tickets = Vec::new();
        tickets
            .try_reserve(cuts.len())
            .map_err(|_| MapError::OutOfMemory)?;
        let mut cleanup = Box::try_new(CleanupNode::empty()).map_err(|_| MapError::OutOfMemory)?;
        {
            let mut allocator = self.allocator.lock();
            if let Err(error) = allocator.reserve_cuts(RegionId::Normal, cuts.len() * 2) {
                return Err(error);
            }
            for (at, len, id) in cuts {
                tickets.push(allocator.retire(RegionId::Normal, at, len, id));
            }
        }
        let mut record = RangeReclaim {
            maps,
            tickets,
            cursor: 0,
        };
        let result = self.finish_range(&mut record);
        if result.is_err() {
            cleanup.record = Some(Reclaim::Range(record));
            self.enqueue_cleanup(cleanup);
        }
        result
    }

    fn finish_range(&self, record: &mut RangeReclaim) -> Result<(), MapError> {
        if self.tree.has_pending_invalidation() {
            self.tree.retry_invalidation()?;
        }
        while record.cursor < record.maps.len() {
            let (map, start, size) = &record.maps[record.cursor];
            let end = start.as_usize() + size;
            let ranges = map.ranges()?;
            for range in ranges {
                let lo = start.as_usize().max(range.start.as_usize());
                let hi = end.min(range.end() + 1);
                if lo < hi {
                    self.tree.unmap(map, VirtAddr::wrap(lo), hi - lo)?;
                }
            }
            record.cursor += 1;
        }
        let mut allocator = self.allocator.lock();
        for ticket in record.tickets.drain(..) {
            allocator.reclaim(RegionId::Normal, ticket);
        }
        Ok(())
    }

    /// 失败（`Err`）时接管那笔映射的清理，映射本身交给 caller 之外的 `Memory`。
    pub(crate) fn dispose(&self, mapping: Mapping) -> Result<(), MapError> {
        self.check_tree_identity(&mapping.reservation)?;
        let id = mapping.reservation.id;
        let mut node = self.allocator.lock().take_cleanup(id);
        if node.is_none() {
            if mapping.map.ranges()?.is_empty() {
                return Ok(());
            }
            return Err(MapError::Shootdown);
        }
        let mut node = node.take().expect("cleanup node");
        node.record = Some(Reclaim::Mapping(mapping));
        let result = match node.record.as_ref().expect("mapping cleanup") {
            Reclaim::Mapping(mapping) => self.unmap(mapping),
            Reclaim::Range(_) => unreachable!(),
        };
        if let Err(error) = result {
            self.enqueue_cleanup(node);
            return Err(error);
        }
        Ok(())
    }

    /// 失败（`Err`）时映射仍在 `mapping` 里，caller 仍持有它。
    pub fn unmap(&self, mapping: &Mapping) -> Result<(), MapError> {
        self.check_tree_identity(&mapping.reservation)?;
        let (start, size) = mapping.map.initial_range();
        if start != mapping.reservation.start || size != mapping.reservation.size {
            return Err(MapError::NoRegion);
        }
        if self.tree.has_pending_invalidation() {
            self.tree.retry_invalidation()?;
        }
        let ranges = mapping.map.ranges()?;
        if ranges.is_empty() {
            self.release_range(&mapping.reservation);
            return Ok(());
        }
        self.check_reservation(&mapping.reservation)?;
        loop {
            let ranges = mapping.map.ranges()?;
            let Some(range) = ranges.first().copied() else {
                self.release_range(&mapping.reservation);
                return Ok(());
            };
            self.tree
                .unmap(&mapping.map, range.start, range.size.get())?;
        }
    }

    fn release_range(&self, reservation: &Reservation) {
        let ticket = self.allocator.lock().retire(
            reservation.region,
            reservation.start.as_usize(),
            reservation.size,
            reservation.id,
        );
        self.allocator.lock().reclaim(reservation.region, ticket);
    }

    fn check_tree_identity(&self, reservation: &Reservation) -> Result<(), MapError> {
        let owner = reservation.tree.upgrade().ok_or(MapError::NoRegion)?;
        if !Arc::ptr_eq(&owner, &self.tree) {
            return Err(MapError::NoRegion);
        }
        Ok(())
    }

    fn check_reservation(&self, reservation: &Reservation) -> Result<(), MapError> {
        let owner = reservation.tree.upgrade().ok_or(MapError::NoRegion)?;
        if !Arc::ptr_eq(&owner, &self.tree)
            || !self
                .allocator
                .lock()
                .has_id(reservation.region, reservation.id)
        {
            return Err(MapError::NoRegion);
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn validate_read(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::R)
    }

    pub(crate) fn validate_write(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::W)
    }

    pub(crate) fn validate_access(&self, va: usize, len: usize, need: PteFlags) -> bool {
        if !Self::user_range(va, len) {
            return false;
        }
        let need = self.pte_policy(need | PteFlags::V);
        self.tree.check_access(VirtAddr::wrap(va), len, need)
    }

    pub(crate) fn instruction_byte(&self, va: usize) -> Option<u8> {
        let mut byte = [0];
        self.copy_from(&mut byte, va, PteFlags::X)
            .then_some(byte[0])
    }

    pub(crate) fn copy_in(&self, dst: &mut [u8], va: usize) -> bool {
        self.copy_from(dst, va, PteFlags::R)
    }

    fn copy_from(&self, dst: &mut [u8], va: usize, need: PteFlags) -> bool {
        if !Self::user_range(va, dst.len()) {
            return false;
        }
        let need = self.pte_policy(need | PteFlags::V);
        self.tree.copy_from(VirtAddr::wrap(va), dst, need)
    }

    pub(crate) fn copy_out(&self, src: &[u8], va: usize) -> bool {
        if !Self::user_range(va, src.len()) {
            return false;
        }
        let need = self.pte_policy(PteFlags::W | PteFlags::V);
        self.tree.copy_to(VirtAddr::wrap(va), src, need)
    }

    /// 用户页错误的完整处置：先还清待办的失效，再按 PTE 与区间元数据判定。
    ///
    /// `true` 表示这次访存已经能继续；`false` 交给入口按原样杀任务。命中但
    /// 权限不足直接判死；命中惰性区间才物化一页，再翻译一次确认。
    pub(crate) fn resolve_fault(&self, addr: VirtAddr, pc: usize, kind: FaultKind) -> bool {
        if self.tree.has_pending_invalidation() {
            if let Err(error) = self.tree.retry_invalidation() {
                error!("page fault waits for invalidation: {:?}", error);
                return false;
            }
        }
        if let Some((_physical, flags)) = self.tree.translate(addr) {
            if satisfies(flags, kind) {
                info!(
                    "page fault resolved by re-walk: {:?} at {:?}",
                    kind, addr
                );
                return true;
            }
            error!(
                "page fault permission violation: {:?} at {:?}",
                kind, addr
            );
            return false;
        }
        if addr.is_user() {
            let Some(map) = self.tree.find(addr) else {
                error!(
                    "unmapped or guard access: {:?} at {:?}, pc={:#x}",
                    kind, addr, pc
                );
                return false;
            };
            let permitted = map.ranges().is_ok_and(|ranges| {
                ranges.iter().any(|range| {
                    range.start <= addr
                        && range.end() >= addr.as_usize()
                        && satisfies(range.current, kind)
                })
            });
            if !permitted {
                error!(
                    "lazy page permission violation: {:?} at {:?}",
                    kind, addr
                );
                return false;
            }
            match self.tree.materialize(&map, addr.page_align(), PAGE_SIZE) {
                Ok(()) => {
                    return self
                        .tree
                        .translate(addr)
                        .is_some_and(|(_, flags)| satisfies(flags, kind));
                }
                Err(error) => {
                    error!("failed to materialize page: {:?}", error);
                    return false;
                }
            }
        }
        error!(
            "kernel page fault: {:?} at {:?}, pc={:#x}",
            kind, addr, pc
        );
        false
    }
}

fn satisfies(flags: PteFlags, kind: FaultKind) -> bool {
    let need = match kind {
        FaultKind::Instruction => PteFlags::X,
        FaultKind::Load => PteFlags::R,
        FaultKind::Store => PteFlags::W,
    };
    flags.contains(PteFlags::V | need)
}

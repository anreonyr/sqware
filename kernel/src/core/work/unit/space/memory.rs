use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;

use riscv::register::satp;

use env::MemoryFail;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr};
use crate::core::memory::flat::{Allocation, BorrowedRegion, Flat, FlatRange, Frame};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{
    Access, ActiveRange, Asid, FaultKind, Nest, NestError, Placement, RegionId, Reservation,
    SpaceKind, View, ViewRange, identity,
};

use super::mapping::Mapping;
use crate::machine::mode;

/// 线外地址空间：只组合地址空间原语，不另存种类、占用或清理责任。
pub struct Memory {
    nest: Arc<Nest>,
}

impl Memory {
    pub(crate) fn new(kind: SpaceKind, kernel: bool) -> Result<Self, NestError> {
        let nest = if kernel {
            Nest::kernel(kind)?
        } else {
            Nest::new(kind)?
        };
        Ok(Self { nest })
    }

    /// 白盒借用：裸地址空间，只给健康用例与调试核对直查，普通路径不得使用。
    #[cfg(debug_assertions)]
    pub(crate) fn backend(&self) -> &Arc<Nest> {
        &self.nest
    }

    pub fn kind(&self) -> SpaceKind {
        self.nest.kind()
    }

    pub(crate) fn asid(&self) -> Asid {
        self.nest.asid()
    }

    pub(crate) fn satp(&self) -> satp::Satp {
        self.nest.satp()
    }

    pub(crate) fn root_ppn(&self) -> usize {
        self.nest.root_ppn()
    }

    pub(crate) fn physical_at(&self, address: VirtAddr) -> Option<PhysAddr> {
        self.nest.physical_at(address)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn audit(&self) {
        self.nest.audit();
    }

    /// basic 口径的线上尺寸：最少 1 字节，向上取整到页；溢出仍是 `Denied`。
    pub(crate) fn basic_bytes(raw: usize) -> Result<usize, MemoryFail> {
        raw.max(1)
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(MemoryFail::Denied)
    }

    /// 非 basic 口径的线上尺寸：非零且按页对齐，否则 `NotAligned`。
    pub(crate) fn exact_bytes(raw: usize) -> Result<usize, MemoryFail> {
        if raw == 0 || !raw.is_multiple_of(PAGE_SIZE) {
            return Err(MemoryFail::NotAligned);
        }
        Ok(raw)
    }

    /// 定点区间：`addr` 必须是页地址，且整段落在用户可达范围内。
    pub(crate) fn fixed_range(addr: VirtAddr, size: usize) -> Result<(), MemoryFail> {
        if addr.offset() != 0 || !Self::user_range(addr.as_usize(), size) {
            return Err(MemoryFail::NotAligned);
        }
        Ok(())
    }

    /// 线上偏移：必须按页对齐，不得让调用方跨页边界泄漏源页前缀。
    pub(crate) fn frame_offset(offset: usize) -> Result<(), MemoryFail> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return Err(MemoryFail::NotAligned);
        }
        Ok(())
    }

    /// 唯一临时来源挂载入口（整段活动形状）：把一段已持有的来源范围挂成普通 `View`。
    ///
    /// 来源必须先准备好（物化或借用凭证都在调用方手里）；
    /// 挂载失败的占用交还责任按 `Nest` 的划分：提交失败交还，已发布失败进清理链。
    ///
    /// 删除条件：PR4 删掉本层时，调用方直接组合 `Nest::reserve` 与 `Nest::attach`。
    pub(crate) fn attach_range(
        &self,
        reservation: Reservation,
        source: FlatRange,
        ceiling: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        Ok(Mapping::new(self.nest.attach(
            reservation,
            source,
            ceiling,
            current,
        )?))
    }

    /// 同一入口的显式活动形状：启动固定跨度与派生投影给出自己的属性与形状。
    pub(crate) fn attach_range_shaped(
        &self,
        reservation: Reservation,
        source: FlatRange,
        attrs: PteFlags,
        active: &[ActiveRange],
    ) -> Result<Mapping, NestError> {
        Ok(Mapping::new(self.nest.attach_shaped(
            reservation,
            source,
            attrs,
            active,
        )?))
    }

    /// 从一段已持有的来源范围选址并挂载：长度由来源范围决定。
    pub(crate) fn map_range(
        &self,
        source: FlatRange,
        align: usize,
        at: Option<VirtAddr>,
        ceiling: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        let size = source
            .page_count()
            .checked_bytes()
            .ok_or(NestError::NotAligned)?;
        let reservation = self.place(size, align, at)?;
        self.attach_range(reservation, source, ceiling, current)
    }

    pub fn user_range(addr: usize, size: usize) -> bool {
        let upper = mode::upper().as_usize();
        addr < upper && addr.checked_add(size).is_some_and(|end| end <= upper)
    }

    /// 旧入口：只在该调用方声明的窗口内登记固定跨度。
    pub(crate) fn reserve(
        &self,
        region: RegionId,
        start: VirtAddr,
        size: usize,
    ) -> Result<Reservation, NestError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        if !self.nest.window_contains(region, start, size) {
            return Err(NestError::NoRegion);
        }
        self.nest.reserve(Placement::exact(region, start, size)?)
    }
    pub(crate) fn allocate_low(
        &self,
        region: RegionId,
        size: usize,
    ) -> Result<Reservation, NestError> {
        self.nest
            .reserve(Placement::lowest(region, size, PAGE_SIZE)?)
    }

    pub(crate) fn allocate_high(
        &self,
        region: RegionId,
        size: usize,
    ) -> Result<Reservation, NestError> {
        self.nest.reserve(Placement::highest(region, size)?)
    }

    pub fn split_reservation(
        &self,
        reservation: Reservation,
        left_size: usize,
    ) -> Result<(Reservation, Reservation), NestError> {
        self.nest.split_reservation(reservation, left_size)
    }

    pub fn release_reservation(&self, reservation: Reservation) -> Result<(), NestError> {
        self.nest.release_reservation(reservation)
    }

    /// 已有关系按初始跨度定位：身份与 generation 都要对得上。
    pub(crate) fn mapping_at(&self, start: VirtAddr, size: usize) -> Result<Mapping, NestError> {
        if size == 0 || start.offset() != 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        let view = self.nest.probe(start).ok_or(NestError::NoRegion)?;
        if view.initial_range() != (start, size) || !view.is_live() {
            return Err(NestError::NoRegion);
        }
        Ok(Mapping::new(view))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn attach_region(
        &self,
        region: RegionId,
        base: usize,
        edge: usize,
    ) -> Result<(), NestError> {
        self.nest.attach_region(region, base, edge)
    }

    pub(crate) fn advance_region(&self, region: RegionId, base: usize) -> Result<(), NestError> {
        self.nest.advance_region(region, base)
    }

    /// 旧提前物化入口：返回前 PTE 必须齐全。
    #[allow(dead_code)]
    pub fn allocate(
        &self,
        layout: Layout,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        self.allocate_eager(
            Self::basic_bytes(layout.size())?,
            layout.align().max(PAGE_SIZE),
            access,
            at,
        )
    }

    pub(crate) fn allocate_bytes(
        &self,
        size: usize,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        self.allocate_eager(Self::basic_bytes(size)?, PAGE_SIZE, access, at)
    }

    #[allow(dead_code)]
    pub fn allocate_lazy(
        &self,
        layout: Layout,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        self.allocate_lazy_core(
            Self::basic_bytes(layout.size())?,
            layout.align().max(PAGE_SIZE),
            limit,
            current,
            at,
        )
    }

    pub(crate) fn allocate_lazy_bytes(
        &self,
        size: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        self.allocate_lazy_core(size, PAGE_SIZE, limit, current, at)
    }

    fn allocate_eager(
        &self,
        size: usize,
        align: usize,
        access: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        // 旧 `Allocate` 仍是 eager：先把来源物化齐，再登记占用并挂载。
        let source =
            Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE).map_err(NestError::from)?)
                .map_err(NestError::from)?
                .whole();
        source.materialize().map_err(NestError::from)?;
        self.nest.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        self.attach_range(reservation, source, Access::all(), access)
            .map_err(MemoryFail::from)
    }

    fn allocate_lazy_core(
        &self,
        size: usize,
        align: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        // 先备好来源，再取占用：来源失败不留下无人接管的地址。
        let source =
            Flat::acquire(Allocation::new(size / PAGE_SIZE, PAGE_SIZE).map_err(NestError::from)?)
                .map_err(NestError::from)?
                .whole();
        self.nest.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        self.attach_range(reservation, source, limit, current)
            .map_err(MemoryFail::from)
    }

    #[allow(dead_code)]
    pub fn view(
        &self,
        source: &Mapping,
        offset: usize,
        layout: Layout,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        self.view_record(
            source.view(),
            offset,
            Self::basic_bytes(layout.size())?,
            layout.align().max(PAGE_SIZE),
            limit,
            current,
            at,
        )
    }

    pub(crate) fn view_record(
        &self,
        source: &Arc<View>,
        offset: usize,
        size: usize,
        align: usize,
        limit: Access,
        current: Access,
        at: Option<VirtAddr>,
    ) -> Result<Mapping, MemoryFail> {
        Self::frame_offset(offset)?;
        let selection = self.whole_or_range(source, offset, size)?;
        self.nest.retry_invalidation()?;
        let reservation = self.place(size, align, at)?;
        self.project(reservation, selection, limit, current)
            .map_err(MemoryFail::from)
    }

    /// 从一段已有关系派生目标关系：源活动与请求窗口的交就是目标的活动形状，
    /// 空洞不被重发布；保留只物化/发布源活动片段的旧副作用。占用由调用方选定。
    ///
    /// PR4 前保留的旧派生桥：Pole 已改从自身来源范围挂载，这里暂时没有生产调用。
    #[allow(dead_code)]
    pub(crate) fn derive_into(
        &self,
        reservation: Reservation,
        source: &Arc<View>,
        offset: usize,
        limit: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        let selection = self.whole_or_range(source, offset, reservation.size())?;
        self.project(reservation, selection, limit, current)
    }

    /// 整段还是窗口：线外只给字节偏移与长度，身份选择在这里就位。
    fn whole_or_range(
        &self,
        source: &Arc<View>,
        offset: usize,
        size: usize,
    ) -> Result<ViewRange, NestError> {
        if offset % PAGE_SIZE != 0 || size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        if offset == 0 && size == source.initial_size() {
            return Ok(source.whole());
        }
        let count = PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?;
        source.range(PageOffset::new(offset / PAGE_SIZE), count)
    }

    /// 按一段真实选择投影目标活动形状并挂载。
    fn project(
        &self,
        reservation: Reservation,
        selection: ViewRange,
        limit: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        let origin = selection.view().clone();
        let origin_nest = origin.owner().ok_or(NestError::NoRegion)?;
        let (lo, _) = selection.window();
        let fragments = selection.active_fragments()?;
        if fragments.is_empty() {
            let _ = self.nest.release_reservation(reservation);
            return Err(NestError::NoRegion);
        }
        // 旧副作用：源活动片段先在 Flat 物化，再回到源关系发布。
        for (start, len) in &fragments {
            origin_nest.materialize_active(self.selection(&origin, *start, *len)?)?;
        }
        let target = reservation.start().as_usize();
        let mut active = Vec::new();
        active
            .try_reserve(fragments.len())
            .map_err(|_| NestError::OutOfMemory)?;
        for (start, len) in fragments {
            active.push(ActiveRange::new(
                VirtAddr::wrap(target + (start.as_usize() - lo)),
                len,
                limit,
                current,
            )?);
        }
        let attrs = self.nest.attrs(reservation.region());
        let source_range = selection.source()?;
        let view = self
            .nest
            .attach_shaped(reservation, source_range, attrs, &active)?;
        Ok(Mapping::new(view))
    }

    pub(crate) fn map_frames(
        &self,
        reservation: Reservation,
        frames: Vec<Arc<Frame>>,
        ceiling: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        if frames.len().checked_mul(PAGE_SIZE) != Some(reservation.size()) {
            let _ = self.nest.release_reservation(reservation);
            return Err(NestError::NotAligned);
        }
        let mut indexed = Vec::new();
        indexed
            .try_reserve_exact(frames.len())
            .map_err(|_| NestError::OutOfMemory)?;
        indexed.extend(
            frames
                .into_iter()
                .enumerate()
                .map(|(index, frame)| (PageIndex::new(index), frame)),
        );
        let source = Flat::from_frames(
            PageCount::new(indexed.len()).expect("nonempty frames"),
            &indexed,
        )?
        .whole();
        self.attach_range(reservation, source, ceiling, current)
    }

    pub(crate) fn map_lazy(
        &self,
        reservation: Reservation,
        ceiling: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        let source =
            Flat::acquire(Allocation::new(reservation.size() / PAGE_SIZE, PAGE_SIZE)?)?.whole();
        self.attach_range(reservation, source, ceiling, current)
    }

    pub(crate) unsafe fn map_borrowed(
        &self,
        reservation: Reservation,
        physical: PhysAddr,
        ceiling: Access,
        current: Access,
    ) -> Result<Mapping, NestError> {
        let region = match unsafe { BorrowedRegion::new(physical, reservation.size()) } {
            Ok(region) => region,
            Err(error) => {
                let _ = self.nest.release_reservation(reservation);
                return Err(error.into());
            }
        };
        let source = Flat::from_region(region)?.whole();
        self.attach_range(reservation, source, ceiling, current)
    }

    /// 启动静态挂载：按 `Exact` 登记真实占用，再按给定属性挂上既有帧。
    pub(crate) fn map_boot_frames(
        &self,
        start: VirtAddr,
        frames: Vec<Arc<Frame>>,
        access: Access,
        attrs: PteFlags,
    ) -> Result<Mapping, NestError> {
        let size = frames.len() * PAGE_SIZE;
        let reservation = self
            .nest
            .reserve(Placement::exact(RegionId::Static, start, size)?)?;
        let mut indexed = Vec::new();
        indexed
            .try_reserve_exact(frames.len())
            .map_err(|_| NestError::OutOfMemory)?;
        indexed.extend(
            frames
                .into_iter()
                .enumerate()
                .map(|(index, frame)| (PageIndex::new(index), frame)),
        );
        let source = Flat::from_frames(
            PageCount::new(indexed.len()).expect("nonempty frames"),
            &indexed,
        )?
        .whole();
        let active = [ActiveRange::new(start, size, access, access)?];
        self.attach_range_shaped(reservation, source, attrs, &active)
    }

    /// 启动静态挂载：把调用方仍保留的物理区间按给定属性挂到已有保留上。
    pub(crate) fn attach_static(
        &self,
        reservation: Reservation,
        region: Arc<BorrowedRegion>,
        offset: usize,
        size: usize,
        access: Access,
        attrs: PteFlags,
    ) -> Result<Mapping, NestError> {
        let start = reservation.start();
        let source = Flat::from_region(region)?.range(
            PageIndex::new(offset / PAGE_SIZE),
            PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?,
        )?;
        let active = [ActiveRange::new(start, size, access, access)?];
        self.attach_range_shaped(reservation, source, attrs, &active)
    }

    /// 启动静态挂载：登记 `Exact` 真实占用后借用调用方保留的物理区间。
    pub(crate) fn map_boot_borrowed(
        &self,
        start: VirtAddr,
        region: Arc<BorrowedRegion>,
        offset: usize,
        size: usize,
        access: Access,
        attrs: PteFlags,
    ) -> Result<Mapping, NestError> {
        let reservation = self
            .nest
            .reserve(Placement::exact(RegionId::Static, start, size)?)?;
        self.attach_static(reservation, region, offset, size, access, attrs)
    }

    /// 把 kernel 团队的跳板映射复制进来；来源由调用方给出。
    pub(crate) fn seed_trampoline(&self, source: &Memory) -> Result<(), NestError> {
        const TRAMPOLINE: VirtAddr = crate::machine::layout::TRAMPOLINE;
        let (physical, flags) = source
            .nest
            .translate(TRAMPOLINE)
            .ok_or(NestError::NoRegion)?;
        let region = unsafe { BorrowedRegion::new(physical, PAGE_SIZE)? };
        let reservation =
            self.nest
                .reserve(Placement::exact(RegionId::Static, TRAMPOLINE, PAGE_SIZE)?)?;
        let source_range = Flat::from_region(region)?.whole();
        let access = Access::from_pte(flags);
        let attrs = flags & (PteFlags::U | PteFlags::G | PteFlags::A | PteFlags::D);
        let active = [ActiveRange::new(TRAMPOLINE, PAGE_SIZE, access, access)?];
        self.attach_range_shaped(reservation, source_range, attrs, &active)?;
        Ok(())
    }

    /// 把一条已有关系的一段权限上限收窄到 `ceiling`。
    pub(crate) fn narrow_record(
        &self,
        source: &Arc<View>,
        start: VirtAddr,
        size: usize,
        ceiling: Access,
    ) -> Result<(), NestError> {
        self.nest
            .narrow(self.selection(source, start, size)?, ceiling)
    }

    /// 把线外的地址长度换成该关系上的身份选择。
    fn selection(
        &self,
        view: &Arc<View>,
        start: VirtAddr,
        size: usize,
    ) -> Result<ViewRange, NestError> {
        if start.offset() != 0 || size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(NestError::NotAligned);
        }
        let offset = start
            .as_usize()
            .checked_sub(view.initial_start().as_usize())
            .ok_or(NestError::NoRegion)?
            / PAGE_SIZE;
        let count = PageCount::new(size / PAGE_SIZE).ok_or(NestError::NotAligned)?;
        view.range(PageOffset::new(offset), count)
    }

    /// 一次保护一个页对齐区间：先整体验完覆盖与上限，再按原次序逐段改权。
    pub fn protect_range(
        &self,
        addr: VirtAddr,
        size: usize,
        access: Access,
    ) -> Result<(), MemoryFail> {
        Self::fixed_range(addr, size)?;
        self.nest.retry_invalidation()?;
        let end = addr.as_usize() + size;
        let ranges = self.nest.active_ranges()?;
        let mut updates = Vec::new();
        updates
            .try_reserve(ranges.len())
            .map_err(|_| MemoryFail::OoM)?;
        let mut covered = addr.as_usize();
        for (view, range) in ranges {
            if range.start.as_usize() >= end || range.end() < addr.as_usize() {
                continue;
            }
            let lo = range.start.as_usize().max(addr.as_usize());
            let hi = (range.end() + 1).min(end);
            if lo != covered {
                return Err(MemoryFail::NoRegion);
            }
            if !range.ceiling.contains(access) {
                return Err(MemoryFail::WidenDenied);
            }
            updates.push((view, VirtAddr::wrap(lo), hi - lo));
            covered = hi;
        }
        if covered != end {
            return Err(MemoryFail::NoRegion);
        }
        for (view, start, size) in updates {
            self.nest
                .set_access(self.selection(&view, start, size)?, access)?;
        }
        if access.contains(Access::EXECUTE) {
            identity::sync_instructions()?;
        }
        Ok(())
    }

    pub(crate) fn mappings_in(
        &self,
        start: VirtAddr,
        size: usize,
    ) -> Result<Vec<Arc<View>>, MemoryFail> {
        Ok(self.nest.views_in(start, size)?)
    }

    pub(crate) fn retry_invalidation(&self) -> Result<(), NestError> {
        self.nest.retry_invalidation()
    }

    /// 旧 VA + 字节口径的撤销：逐段换成身份选择，交核心按段退休/收回。
    pub(crate) fn unmap_range(&self, start: VirtAddr, size: usize) -> Result<(), NestError> {
        if !Self::user_range(start.as_usize(), size)
            || start.offset() != 0
            || !size.is_multiple_of(PAGE_SIZE)
        {
            return Err(NestError::NotAligned);
        }
        let end = start.as_usize() + size;
        let mut selections = Vec::new();
        for (view, range) in self.nest.active_ranges()? {
            let lo = start.as_usize().max(range.start.as_usize());
            if range.start.as_usize() >= end || range.end() < start.as_usize() {
                continue;
            }
            let hi = (range.end() + 1).min(end);
            if lo < hi {
                selections.push(self.selection(&view, VirtAddr::wrap(lo), hi - lo)?);
            }
        }
        if selections.is_empty() {
            return Err(NestError::NoRegion);
        }
        for selection in selections {
            self.nest.detach(selection)?;
        }
        Ok(())
    }

    /// 借出句柄的整段关闭：失败时 `mapping` 仍在 caller 手里。
    pub fn unmap(&self, mapping: &Mapping) -> Result<(), NestError> {
        self.nest.detach(mapping.view().whole())
    }

    /// 消费式接管：拆开兼容壳，把责任交给 Nest。
    pub(crate) fn dispose(&self, mapping: Mapping) -> Result<(), NestError> {
        self.nest.dispose(mapping.into_view())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn validate_read(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, Access::READ)
    }

    pub(crate) fn validate_write(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, Access::WRITE)
    }

    pub(crate) fn validate_access(&self, va: usize, len: usize, need: Access) -> bool {
        Self::user_range(va, len) && self.nest.check_access(VirtAddr::wrap(va), len, need)
    }

    pub(crate) fn instruction_byte(&self, va: usize) -> Option<u8> {
        let mut byte = [0];
        self.copy_from(&mut byte, va, Access::EXECUTE)
            .then_some(byte[0])
    }

    pub(crate) fn copy_in(&self, dst: &mut [u8], va: usize) -> bool {
        self.copy_from(dst, va, Access::READ)
    }

    fn copy_from(&self, dst: &mut [u8], va: usize, need: Access) -> bool {
        Self::user_range(va, dst.len()) && self.nest.copy_from(VirtAddr::wrap(va), dst, need)
    }

    pub(crate) fn copy_out(&self, src: &[u8], va: usize) -> bool {
        Self::user_range(va, src.len()) && self.nest.copy_to(VirtAddr::wrap(va), src, Access::WRITE)
    }

    pub(crate) fn resolve_fault(&self, addr: VirtAddr, pc: usize, kind: FaultKind) -> bool {
        self.nest.resolve_fault(addr, pc, kind)
    }

    /// 选址：`at` 为空时取满足 `align` 的最低空档，否则要它自己也满足 `align`。
    fn place(
        &self,
        size: usize,
        align: usize,
        at: Option<VirtAddr>,
    ) -> Result<Reservation, NestError> {
        let Some(addr) = at else {
            return self
                .nest
                .reserve(Placement::lowest(RegionId::Normal, size, align)?);
        };
        if addr.as_usize() % align != 0 {
            return Err(NestError::NotAligned);
        }
        self.reserve(RegionId::Normal, addr, size)
    }
}

/// 地址空间失败折成线上码：`Denied` 兼收 `Pinned`、`NotMapped` 与 `DramOverlap`。
impl From<NestError> for MemoryFail {
    fn from(error: NestError) -> Self {
        match error {
            NestError::OutOfMemory => MemoryFail::OoM,
            NestError::NotAligned => MemoryFail::NotAligned,
            NestError::NoRegion => MemoryFail::NoRegion,
            NestError::AlreadyMapped => MemoryFail::AlreadyMapped,
            NestError::WidenDenied => MemoryFail::WidenDenied,
            NestError::Shootdown => MemoryFail::Busy,
            NestError::Pinned => MemoryFail::Denied,
            NestError::NotMapped | NestError::DramOverlap => MemoryFail::Denied,
        }
    }
}

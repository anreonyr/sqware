//! IOMMU 那一域的核心：一个共享引擎、一条私有 Domain 集合、每个 Domain 一笔当前绑定。
//!
//! 五步仍是独立效果：取得私有 IOVA 与帧引用／发布／撤销／关队列／解除。帧引用只在本
//! Domain 的 `Holds` 上有一个持有方，只在完整停链完成之后放下。共享的命令队列、故障
//! 队列、DDT 与完成记录由本 Core 唯一升降；一个 Domain 结清不拆另一个 Domain 的翻译。

use alloc::boxed::Box;
use alloc::vec::Vec;
use env::{Access, AcquireSource, PAGE_SIZE, Permission, PieKind, PieToken, ReferenceRange, TaskId};
use execution::memory::traverse;
use hub_api::BindingId;
use resource::dock::Dock;

use crate::driver::iommu::acquire::AcquireRequest;
use crate::driver::iommu::api;
use crate::driver::iommu::binding::Binding;
use crate::driver::iommu::context;
use crate::driver::iommu::domain::{Domain, FIRST_IOVA, IOVA_LIMIT, IOVA_STRIDE};
use crate::driver::iommu::domain_id::DomainId;
use crate::driver::iommu::domain_range::DomainRange;
use crate::driver::iommu::engine::Engine;
use crate::driver::iommu::fence::Fence;
use crate::driver::iommu::handshake::Handshake;
use crate::driver::iommu::hold::{
    Close as HoldClose, DeviceRange, Failure as HoldFailure, Registration, Scope,
};
use crate::driver::iommu::invalidation::Invalidation;
use crate::driver::iommu::layout::{self, COMMANDS, DDT, FAULTS, FENCE, PAGES};
use crate::driver::iommu::order;
use crate::driver::iommu::pages::Pages;
use crate::driver::iommu::phase::Phase;
use crate::driver::iommu::queue::{self, CommandQueue};
use crate::driver::iommu::references::References;
use crate::driver::iommu::regs;
use crate::driver::iommu::settled::Settled;
use crate::driver::iommu::table;
use crate::driver::shared::dma;
use crate::driver::shared::platform::Window;

/// 一次私有映射最多 2 MiB（与一个 2 MiB 私有 IOVA 槽同形）。
const MAX_BYTES: u64 = 1 << 21;
/// 升降共享队列／DDTP 的同一条有界握手期限。
const ENGINE_NS: u64 = 2_000_000_000;

pub struct Core {
    /// 寄存器窗那一份映射（活到本对象最后一次访问之后）。
    _region: Dock,
    base: usize,
    /// 本实例的实际引擎任务号：`DomainId.engine` 的来源。
    engine: TaskId,
    /// 本实例的保留授权（声明供给交来的）。
    retention: PieToken,
    /// 本域那一窗保留页（DDT／页表／队列／完成记录）。
    pages: Pages,
    /// 唯一提交者：命令与故障队列、完成记录页。
    queues: CommandQueue,
    /// 持有账要的那两步效果（本任务表上的 Acquire/Release）。
    references: References,
    /// 私有 Domain 全集；没有第二本表。
    domains: Vec<Domain>,
    /// 下一个 Domain 键；用尽后不环绕。
    next_key: Option<u64>,
    /// 共享引擎的物理态。
    mode: Engine,
}

impl Core {
    /// 起手：取保留页、立两台队列，并核对设备目录表初态是 Off。
    pub fn start(region: Window, retention: PieToken) -> Result<Self, &'static str> {
        let dock = Dock::open(region.token).map_err(|_| "iommu: region mapping")?;
        let base = dock.view().base();
        let initial = regs::read64(base, regs::DDTP);
        if initial & 0xF != regs::DDTP_MODE_OFF || initial & regs::DDTP_BUSY != 0 {
            return Err("iommu: ddtp not off at start");
        }
        let (commands, faults) = (
            regs::read32(base, regs::CQCSR),
            regs::read32(base, regs::FQCSR),
        );
        let requests = regs::read32(base, regs::PQCSR);
        if ![commands, faults, requests].into_iter().all(|csr| {
            queue::quiet(csr, regs::QUEUE_ON)
                && csr & regs::QUEUE_ENABLE == 0
                && !queue::queue_error(csr)
        }) || queue::command_error(commands)
        {
            return Err("iommu: queues not off at start");
        }
        let mut pages = Pages::default();
        if let Err(failure) = pages.take(retention, PAGES) {
            if let Err(cleanup) = pages.close() {
                crate::debug::put(&alloc::format!(
                    "iommu: startup failed {failure}; cleanup unsettled {cleanup}"
                ));
                return Err(cleanup);
            }
            return Err(failure);
        }
        let queues = CommandQueue::new(base, &pages, COMMANDS, FAULTS, FENCE);
        crate::debug::put(&alloc::format!(
            "iommu: space task={} iommu={base:#x} retain={} ddtp={initial:#x} (off)",
            env::unit::self_id().get(),
            retention.get(),
        ));
        crate::debug::put(&alloc::format!(
            "iommu: retained engine pool pages={} contexts={}",
            pages.count(),
            context::CAPACITY
        ));
        Ok(Self {
            _region: dock,
            base,
            engine: env::unit::self_id(),
            retention,
            pages,
            queues,
            references: References,
            domains: Vec::new(),
            next_key: Some(1),
            mode: Engine::Off,
        })
    }

    pub fn base(&self) -> usize {
        self.base
    }

    /// 此刻所有 Domain 还持有几页帧（协调入口与场景读用）。
    pub fn held(&self) -> u32 {
        self.domains
            .iter()
            .filter_map(|domain| {
                let binding = domain.binding.as_ref()?;
                binding.hold?;
                domain
                    .holds
                    .reference(binding.range.range)
                    .map(|_| binding.pages)
            })
            .sum::<usize>()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// 本 Domain 当前私有分配的真实字节数。
    pub fn range(&self, domain: DomainId) -> Option<(u64, u64)> {
        let domain = self.domains.iter().find(|held| held.id() == domain)?;
        let binding = domain.binding.as_ref()?;
        Some((binding.range.range.base(), binding.range.range.length()))
    }

    /// 建立（或取回）实际 Driver 的私有 Domain；同一 requester 不能换主人。
    pub fn establish(
        &mut self,
        caller: TaskId,
        requester: u32,
    ) -> Result<DomainId, api::Fail> {
        let requester = u16::try_from(requester)
            .ok()
            .filter(|requester| context::offset(*requester).is_some())
            .ok_or(api::Fail::Denied)?;
        if let Some(domain) = self
            .domains
            .iter()
            .find(|domain| domain.requester == requester)
        {
            return if domain.owner == caller {
                Ok(domain.id())
            } else {
                Err(api::Fail::Denied)
            };
        }
        let key = self.next_key.filter(|key| *key != 0).ok_or(api::Fail::Full)?;
        let next = key.checked_add(1);
        let id = DomainId::new(self.engine, key);
        self.domains
            .try_reserve(1)
            .map_err(|_| api::Fail::Unavailable)?;
        self.domains.push(Domain::new(id, caller, requester));
        self.next_key = next;
        crate::debug::put(&alloc::format!(
            "iommu: domain {} established owner={} requester={}",
            id.key(),
            caller.get(),
            requester,
        ));
        Ok(id)
    }

    /// ① 取得私有 IOVA 与帧引用：分配一次、先落记录、再取引用。
    pub fn acquire(&mut self, caller: TaskId, request: AcquireRequest) -> Result<(), api::Fail> {
        let index = self.domain_index(caller, request.domain)?;
        Self::validate_binding(request.binding)?;
        if request.execution == TaskId::new(0) || request.source == PieToken::NONE {
            return Err(api::Fail::Denied);
        }
        self.freeze_hub(index, request.binding.hub)?;

        {
            let domain = &self.domains[index];
            if let Some(binding) = domain.binding.as_ref() {
                if binding.binding == request.binding
                    && binding.execution == request.execution
                    && binding.source == request.source
                    && binding.access == request.access
                {
                    return self.finish_acquire(index);
                }
                return Err(api::Fail::Conflict);
            }
            if request.binding.serial <= domain.retired {
                return Err(api::Fail::Conflict);
            }
        }

        let access = Access::from_bits(request.access).ok_or(api::Fail::Denied)?;
        if access == Access::NONE {
            return Err(api::Fail::Denied);
        }
        let requester = self.domains[index].requester;
        let base = self.domains[index].next_iova;
        if self.adopted(request.source) {
            return Err(api::Fail::Denied);
        }
        let source = resource::raw::inspect(request.source).map_err(|_| api::Fail::NotReady)?;
        if !source.alive || source.kind != PieKind::Pole {
            return Err(api::Fail::Denied);
        }
        if source.permission != Permission::FETCH.union(Permission::STORE) {
            return Err(api::Fail::Denied);
        }
        if source.vestor != request.execution {
            return Err(api::Fail::Denied);
        }
        if source.mark != dma::window_mark(request.binding.serial) {
            return Err(api::Fail::Denied);
        }
        let dock = Dock::open(request.source).map_err(|_| api::Fail::Denied)?;
        let bytes = dock.view().size() as u64;
        dock.shut().map_err(|_| api::Fail::Denied)?;
        if bytes == 0 || bytes > MAX_BYTES || !(bytes as usize).is_multiple_of(PAGE_SIZE) {
            return Err(api::Fail::Denied);
        }
        let pages = (bytes / PAGE_SIZE as u64) as usize;
        let next = base.checked_add(IOVA_STRIDE).ok_or(api::Fail::Full)?;
        let end = base.checked_add(bytes).ok_or(api::Fail::Full)?;
        if end > IOVA_LIMIT || base < FIRST_IOVA {
            return Err(api::Fail::Full);
        }
        let range = DeviceRange::new(base, bytes).ok_or(api::Fail::Denied)?;
        let mut physical = Vec::new();
        physical
            .try_reserve_exact(pages)
            .map_err(|_| api::Fail::Unavailable)?;
        physical.resize(pages, 0);

        // Reserve the pair before taking the frame reference: a failed ticketing
        // attempt leaves no reference and does not consume the IOVA cursor.
        let publish_slot = 2 * usize::from(requester);
        let retire_slot = publish_slot + 1;
        let (publish, retire) = self
            .queues
            .tickets(publish_slot, retire_slot)
            .ok_or(api::Fail::Unavailable)?;
        let pscid = u32::from(requester) + 1;
        let publish = Invalidation::new(requester, pscid, publish).ok_or(api::Fail::Denied)?;
        let settled = Settled::new(
            range,
            self.pages.at(DDT, 0) as *const u64,
            requester,
            &retire,
        );
        let retire = Invalidation::new(requester, pscid, retire).ok_or(api::Fail::Denied)?;
        let domain = &mut self.domains[index];
        if domain
            .holds
            .attach(Scope::new(
                range,
                request.execution,
                Some(Box::new(settled)),
            ))
            .is_err()
        {
            return Err(api::Fail::Conflict);
        }
        // Persist the allocation and binding before any fallible reference work.
        domain.next_iova = next;
        domain.binding = Some(Binding {
            binding: request.binding,
            hold: None,
            requester,
            execution: request.execution,
            range: DomainRange {
                domain: request.domain,
                range,
            },
            pages,
            access: request.access,
            source: request.source,
            physical,
            publish,
            retire,
            programmed: false,
            phase: Phase::Acquiring,
            failure: None,
        });
        self.finish_acquire(index)
    }

    fn finish_acquire(&mut self, index: usize) -> Result<(), api::Fail> {
        let Self {
            domains,
            references,
            retention,
            ..
        } = self;
        let Domain {
            owner,
            holds,
            binding,
            ..
        } = &mut domains[index];
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        match binding.phase {
            Phase::Acquired => return Ok(()),
            Phase::Acquiring => {}
            _ => return Err(api::Fail::Conflict),
        }
        if binding.hold.is_none() {
            let id = holds
                .register(
                    Registration {
                        range: binding.range.range,
                        source: AcquireSource::Source {
                            token: binding.source,
                            offset: 0,
                            count: binding.pages,
                        },
                        retention: *retention,
                    },
                    references,
                )
                .map_err(|_| api::Fail::Denied)?;
            binding.hold = Some(id);
        }
        let handle = holds
            .reference(binding.range.range)
            .ok_or(api::Fail::Unavailable)?;
        let window =
            ReferenceRange::new(handle, 0, binding.pages).ok_or(api::Fail::Unavailable)?;
        traverse(*retention, window, &mut binding.physical)
            .map_err(|_| api::Fail::Unavailable)?;
        binding.phase = Phase::Acquired;
        crate::debug::put(&alloc::format!(
            "iommu: binding {} acquired requester={} pages={} iova={:#x}",
            binding.binding.serial,
            binding.requester,
            binding.pages,
            binding.range.range.base(),
        ));
        crate::debug::put(&alloc::format!(
            "iommu: domain {} binding {} owner={} requester={} iova={:#x} bytes={}",
            binding.range.domain.key(),
            binding.binding.serial,
            owner.get(),
            binding.requester,
            binding.range.range.base(),
            binding.range.range.length(),
        ));
        Ok(())
    }

    /// ② 发布：先在共享引擎上抬队列，再写本 Domain 的目录项与叶项，最后走本 Domain
    /// 的 DIR→VMA→IOFENCE。另一个 Domain 已 On 时不重置共享队列与 cookie 发行器。
    pub fn publish(
        &mut self,
        caller: TaskId,
        domain: DomainId,
        binding: BindingId,
    ) -> Result<::schedule::Progress, api::Fail> {
        let index = self.domain_index(caller, domain)?;
        Self::validate_binding(binding)?;
        self.freeze_hub(index, binding.hub)?;
        self.current_binding(index, binding)?;
        match self.domains[index].binding.as_ref().map(|binding| binding.phase) {
            Some(Phase::Published) => return Ok(::schedule::Progress::Done),
            Some(Phase::Acquired) | Some(Phase::Publishing) => {}
            _ => return Err(api::Fail::Conflict),
        }
        match self.mode {
            Engine::Lowering { .. } => return Ok(::schedule::Progress::Pending),
            Engine::Raising => {
                self.fail(index, api::PUBLISH);
                return Err(api::Fail::Unavailable);
            }
            Engine::Off => {
                if let Err(fail) = self.raise_engine() {
                    self.fail(index, api::PUBLISH);
                    return Err(fail);
                }
            }
            Engine::On => {}
        }
        if self.domains[index]
            .binding
            .as_ref()
            .is_some_and(|binding| binding.phase == Phase::Acquired)
        {
            self.domains[index]
                .binding
                .as_mut()
                .expect("binding present")
                .phase = Phase::Publishing;
        }

        let Self {
            domains,
            queues,
            pages,
            base,
            ..
        } = self;
        let Domain {
            holds, binding, ..
        } = &mut domains[index];
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        if !binding.programmed {
            let id = binding.hold.ok_or(api::Fail::Unavailable)?;
            if holds.publish_begin(id).is_err() {
                binding.failure = Some((api::PARTY, api::PUBLISH));
                return Err(api::Fail::Unavailable);
            }
            binding.programmed = true;
            let hardware = (|| -> Result<(), api::Fail> {
                let seen = regs::read64(*base, regs::DDTP);
                if seen & 0xF != regs::DDTP_MODE_OFF
                    && (seen & 0xF != regs::DDTP_MODE_1LVL || seen & regs::DDTP_BUSY != 0)
                {
                    return Err(api::Fail::Unavailable);
                }
                let (root, middle, leaf) =
                    layout::tables(binding.requester).ok_or(api::Fail::Denied)?;
                for page in [root, middle, leaf] {
                    pages.fill(page, 0);
                }
                let context =
                    context::DeviceContext::first_stage(pages.physical(root), u32::from(binding.requester) + 1)
                        .ok_or(api::Fail::Denied)?;
                let offset = context::offset(binding.requester).ok_or(api::Fail::Denied)?;
                for (index, word) in context.words().iter().enumerate() {
                    pages.put64(DDT, offset + index * 8, *word);
                }
                table::map_physical(
                    pages,
                    root,
                    middle,
                    leaf,
                    binding.range.range.base() as usize,
                    &binding.physical,
                )
                .map_err(|_| api::Fail::Denied)?;
                order::sync();
                let root = pages.physical(DDT);
                let current = regs::read64(*base, regs::DDTP);
                let same = current & 0xF == regs::DDTP_MODE_1LVL
                    && current & regs::DDTP_BUSY == 0
                    && (current & regs::PPN_FIELD) >> regs::PPN_SHIFT == regs::page_number(root);
                if !same {
                    regs::ddtp_set(*base, regs::DDTP_MODE_1LVL, Some(root))
                        .map_err(|_| api::Fail::Unavailable)?;
                }
                Ok(())
            })();
            let ended = holds.publish_end(
                id,
                hardware
                    .as_ref()
                    .map(|_| ())
                    .map_err(|_| HoldFailure::Denied),
            );
            if let Err(fail) = hardware {
                binding.phase = Phase::PublishFailed;
                binding.failure = Some((api::PARTY, api::PUBLISH));
                return Err(fail);
            }
            if ended.is_err() {
                binding.phase = Phase::PublishFailed;
                binding.failure = Some((api::PARTY, api::PUBLISH));
                return Err(api::Fail::Unavailable);
            }
        }
        match binding.publish.step(queues) {
            Fence::Settled => {
                binding.phase = Phase::Published;
                crate::debug::put(&alloc::format!(
                    "iommu: binding {} published requester={} held={} ddtp=0x{:x}",
                    binding.binding.serial,
                    binding.requester,
                    binding.pages,
                    regs::read64(*base, regs::DDTP),
                ));
                Ok(::schedule::Progress::Done)
            }
            Fence::Waiting => Ok(::schedule::Progress::Pending),
            Fence::Expired => {
                binding.failure = Some((api::PARTY, api::PUBLISH));
                Err(api::Fail::Unavailable)
            }
        }
    }

    /// ③ 撤销本 Domain 的翻译：清它自己那格 DC 并回读，再走本 Domain 的 retire。
    /// 只证明本步骤：不释放帧，也不宣称端点已排空。
    pub fn invalidate(
        &mut self,
        caller: TaskId,
        domain: DomainId,
        binding: BindingId,
    ) -> Result<::schedule::Progress, api::Fail> {
        let index = self.domain_index(caller, domain)?;
        Self::validate_binding(binding)?;
        self.freeze_hub(index, binding.hub)?;
        if self.domains[index].binding.is_none() {
            // Invalidate is the first provider step in the close-before-Acquire
            // path: tombstone this exact full binding, then Quiesce/Release are
            // idempotent. A different id against a current binding stays Unknown.
            self.domains[index].retired = self.domains[index].retired.max(binding.serial);
            return Ok(::schedule::Progress::Done);
        }
        self.current_binding(index, binding)?;
        let Self {
            domains,
            queues,
            pages,
            ..
        } = self;
        let Domain { binding, .. } = &mut domains[index];
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        match binding.phase {
            Phase::Publishing if !binding.programmed => {
                // Queue enable failed before publication; close owns the partial queues.
                binding.phase = Phase::Invalidated;
                return Ok(::schedule::Progress::Done);
            }
            Phase::Publishing | Phase::PublishFailed | Phase::Published => {
                binding.phase = Phase::Invalidating
            }
            Phase::Invalidating => {}
            Phase::Invalidated
            | Phase::Quiescing
            | Phase::Quiet
            | Phase::Releasing
            | Phase::Released => return Ok(::schedule::Progress::Done),
            Phase::Acquiring | Phase::Acquired => {
                binding.phase = Phase::Invalidated;
                return Ok(::schedule::Progress::Done);
            }
        }
        if binding.programmed {
            let at = context::offset(binding.requester).ok_or(api::Fail::Denied)?;
            for index in 0..8 {
                pages.put64(DDT, at + index * 8, 0);
            }
            let clean = (0..8).all(|index| pages.get64(DDT, at + index * 8) == 0);
            if !clean {
                binding.failure = Some((api::PARTY, api::INVALIDATE));
                return Err(api::Fail::Unavailable);
            }
            order::sync();
            binding.programmed = false;
        }
        match binding.retire.step(queues) {
            Fence::Settled => {
                binding.phase = Phase::Invalidated;
                crate::debug::put(&alloc::format!(
                    "iommu: binding {} invalidated frames held={} (not released)",
                    binding.binding.serial,
                    binding.pages,
                ));
                Ok(::schedule::Progress::Done)
            }
            Fence::Waiting => Ok(::schedule::Progress::Pending),
            Fence::Expired => {
                if binding.failure != Some((api::PARTY, api::INVALIDATE)) {
                    crate::debug::put(&alloc::format!(
                        "iommu: binding {} invalidate fence expired frames held={}",
                        binding.binding.serial,
                        binding.pages,
                    ));
                }
                binding.failure = Some((api::PARTY, api::INVALIDATE));
                Err(api::Fail::Unavailable)
            }
        }
    }

    /// ④ 认证 Driver 在设备停止之后发来的 Quiesce 就是停止授权。另一个实际引擎
    /// 用户仍在时本绑定只进 Quiet；最后一张停稳的映射在放下帧引用之前把共享引擎
    /// 实际降到 Off，Pending 期间保持 Quiescing 并保留数据引用。
    pub fn quiesce(
        &mut self,
        caller: TaskId,
        domain: DomainId,
        binding: BindingId,
    ) -> Result<::schedule::Progress, api::Fail> {
        let index = self.domain_index(caller, domain)?;
        Self::validate_binding(binding)?;
        self.freeze_hub(index, binding.hub)?;
        if self.domains[index].binding.is_none() {
            return if binding.serial <= self.domains[index].retired {
                Ok(::schedule::Progress::Done)
            } else {
                Err(api::Fail::Unknown)
            };
        }
        self.current_binding(index, binding)?;
        let serial = binding.serial;
        let phase = {
            let binding = self.domains[index]
                .binding
                .as_mut()
                .ok_or(api::Fail::Unknown)?;
            match binding.phase {
                Phase::Invalidated => binding.phase = Phase::Quiescing,
                Phase::Quiescing => {}
                Phase::Quiet | Phase::Releasing | Phase::Released => {
                    return Ok(::schedule::Progress::Done);
                }
                _ => return Err(api::Fail::Conflict),
            }
            binding.phase
        };
        debug_assert_eq!(phase, Phase::Quiescing);

        if self.other_engine_users(index) {
            self.domains[index]
                .binding
                .as_mut()
                .expect("binding present")
                .phase = Phase::Quiet;
            let (commands, faults) = self.queues.status();
            let requests = regs::read32(self.base, regs::PQCSR);
            let (recorded, _, fault_did, _) = self.queues.faults();
            crate::debug::put(&alloc::format!(
                "iommu: binding {serial} quiesced shared engine on cqcsr=0x{commands:x} fqcsr=0x{faults:x} pqcsr=0x{requests:x} queue_enable={} faults={recorded} last_did={fault_did} engine_off=false",
                (commands & regs::QUEUE_ENABLE) | (faults & regs::QUEUE_ENABLE),
            ));
            return Ok(::schedule::Progress::Done);
        }

        match self.lower_engine() {
            Ok(::schedule::Progress::Done) => {}
            Ok(::schedule::Progress::Pending) => return Ok(::schedule::Progress::Pending),
            Err(fail) => {
                self.domains[index]
                    .binding
                    .as_mut()
                    .expect("binding present")
                    .failure = Some((api::PARTY, api::QUIESCE));
                return Err(fail);
            }
        }
        let (commands, faults) = self.queues.status();
        let requests = regs::read32(self.base, regs::PQCSR);
        let (recorded, _, fault_did, _) = self.queues.faults();
        let binding = self.domains[index]
            .binding
            .as_mut()
            .expect("binding present");
        binding.phase = Phase::Quiet;
        crate::debug::put(&alloc::format!(
            "iommu: binding {serial} queues stopped cqcsr=0x{commands:x} fqcsr=0x{faults:x} pqcsr=0x{requests:x} queue_enable={} faults={recorded} last_did={fault_did}",
            (commands & regs::QUEUE_ENABLE) | (faults & regs::QUEUE_ENABLE),
        ));
        Ok(::schedule::Progress::Done)
    }

    /// ⑤ 解除本 Domain 的引用与本地来源窗；最后一张停稳的映射在放下数据引用之前
    /// 已把共享引擎实际降到 Off，Release 再回读确认（正常收场不调用 `shutdown`）。
    pub fn release(
        &mut self,
        caller: TaskId,
        domain: DomainId,
        binding: BindingId,
        reason: u8,
    ) -> Result<::schedule::Progress, api::Fail> {
        let index = self.domain_index(caller, domain)?;
        Self::validate_binding(binding)?;
        let reason = match reason {
            value if value == hub_api::Close::Normal.code() => HoldClose::Normal,
            value if value == hub_api::Close::Fault.code() => HoldClose::Fault,
            _ => return Err(api::Fail::Denied),
        };
        self.freeze_hub(index, binding.hub)?;
        if self.domains[index].binding.is_none() {
            self.report_release(binding.serial, !matches!(self.mode, Engine::Off))?;
            let retired = self.domains[index].retired.max(binding.serial);
            self.domains[index].retired = retired;
            return Ok(::schedule::Progress::Done);
        }
        self.current_binding(index, binding)?;
        if !matches!(
            self.domains[index]
                .binding
                .as_ref()
                .map(|binding| binding.phase),
            Some(Phase::Quiescing | Phase::Quiet | Phase::Releasing | Phase::Released)
        ) {
            return Err(api::Fail::Conflict);
        }

        // The last mapping lowers and proves actual Off before the Reference is
        // released. Other live engine users keep the shared queues On.
        let shared = self.other_engine_users(index);
        if !shared {
            match self.lower_engine()? {
                ::schedule::Progress::Done => {}
                ::schedule::Progress::Pending => return Ok(::schedule::Progress::Pending),
            }
        }
        if self.domains[index]
            .binding
            .as_ref()
            .is_some_and(|binding| binding.phase == Phase::Quiescing)
        {
            self.domains[index]
                .binding
                .as_mut()
                .expect("binding present")
                .phase = Phase::Quiet;
        }
        self.release_binding(index, reason)?;
        self.report_release(binding.serial, shared)?;
        let domain = &mut self.domains[index];
        domain.binding = None;
        domain.retired = domain.retired.max(binding.serial);
        if shared {
            crate::debug::put(&alloc::format!(
                "iommu: binding {} settled, binding free while shared engine on",
                binding.serial
            ));
        } else {
            crate::debug::put(&alloc::format!(
                "iommu: binding {} settled, device free for a new claim",
                binding.serial
            ));
        }
        Ok(::schedule::Progress::Done)
    }

    fn report_release(&self, serial: u64, shared: bool) -> Result<(), api::Fail> {
        if !shared { self.verify_off()?; }
        let (commands, faults) = self.queues.status();
        let requests = regs::read32(self.base, regs::PQCSR);
        let stopped = regs::read64(self.base, regs::DDTP);
        let enable = (commands & regs::QUEUE_ENABLE) | (faults & regs::QUEUE_ENABLE);
        if shared {
            crate::debug::put(&alloc::format!(
                "iommu: binding {serial} released shared ddtp=0x{stopped:x} cqcsr=0x{commands:x} fqcsr=0x{faults:x} pqcsr=0x{requests:x} queue_enable={enable} frames=0 engine_off=false",
            ));
        } else {
            crate::debug::put(&alloc::format!(
                "iommu: binding {serial} released ddtp=0x{stopped:x} cqcsr=0x{commands:x} fqcsr=0x{faults:x} pqcsr=0x{requests:x} queue_enable={enable} frames=0 stopped=true",
            ));
        }
        Ok(())
    }

    fn release_binding(&mut self, index: usize, reason: HoldClose) -> Result<(), api::Fail> {
        let Self {
            domains,
            references,
            ..
        } = self;
        let Domain {
            holds, binding, ..
        } = &mut domains[index];
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        match binding.phase {
            Phase::Quiet => {
                let _ = holds.close(binding.range.range, reason);
                binding.phase = Phase::Releasing;
            }
            Phase::Releasing => {}
            Phase::Released => return Ok(()),
            _ => return Err(api::Fail::Conflict),
        }
        let _ = holds.retry(references);
        if holds.failure(binding.range.range).is_some() {
            binding.failure = Some((api::PARTY, api::RELEASE));
            return Err(api::Fail::Unavailable);
        }
        if binding.source != PieToken::NONE && resource::raw::alive(binding.source) {
            if env::pie::release(binding.source, env::ReleaseMode::Revoke).is_err() {
                binding.failure = Some((api::PARTY, api::RELEASE));
                return Err(api::Fail::Unavailable);
            }
        }
        binding.source = PieToken::NONE;
        if !holds.forget(binding.range.range) {
            return Err(api::Fail::Unavailable);
        }
        binding.phase = Phase::Released;
        Ok(())
    }

    /// 收场：所有 Domain 都已结清、共享引擎实际 Off 后才关本域保留页。
    pub fn shutdown(&mut self) -> Result<(), &'static str> {
        if self.domains.iter().any(|domain| {
            domain.binding.is_some() || domain.holds.occupied().next().is_some()
        }) {
            return Err("iommu: binding not settled");
        }
        match self.lower_engine() {
            Ok(::schedule::Progress::Done) => {}
            Ok(::schedule::Progress::Pending) => return Err("iommu: engine lowering"),
            Err(_) => return Err("iommu: engine not off"),
        }
        self.verify_off().map_err(|_| "iommu: engine not off")?;
        self.pages.close()?;
        crate::debug::put("iommu: pages closed");
        Ok(())
    }

    fn domain_index(&self, caller: TaskId, domain: DomainId) -> Result<usize, api::Fail> {
        if domain.engine() != self.engine {
            return Err(api::Fail::Denied);
        }
        let index = self
            .domains
            .iter()
            .position(|held| held.id().key() == domain.key())
            .ok_or(api::Fail::Unknown)?;
        if self.domains[index].owner != caller {
            return Err(api::Fail::Denied);
        }
        Ok(index)
    }

    fn freeze_hub(&mut self, index: usize, hub: TaskId) -> Result<(), api::Fail> {
        let domain = &mut self.domains[index];
        match domain.hub {
            Some(held) if held == hub => Ok(()),
            Some(_) => Err(api::Fail::Denied),
            None => {
                domain.hub = Some(hub);
                Ok(())
            }
        }
    }

    fn validate_binding(binding: BindingId) -> Result<(), api::Fail> {
        if binding.hub.get() == 0 || binding.serial == 0 {
            return Err(api::Fail::Denied);
        }
        Ok(())
    }

    fn current_binding(&self, index: usize, binding: BindingId) -> Result<(), api::Fail> {
        match self.domains[index].binding.as_ref() {
            Some(current) if current.binding == binding => Ok(()),
            _ => Err(api::Fail::Unknown),
        }
    }

    fn fail(&mut self, index: usize, operation: u8) {
        if let Some(binding) = self.domains[index].binding.as_mut() {
            binding.failure = Some((api::PARTY, operation));
        }
    }

    fn adopted(&self, source: PieToken) -> bool {
        self.domains.iter().any(|domain| {
            domain
                .binding
                .as_ref()
                .is_some_and(|binding| binding.source == source)
        })
    }

    fn raise_engine(&mut self) -> Result<(), api::Fail> {
        match self.mode {
            Engine::On => return Ok(()),
            Engine::Off => {}
            _ => return Err(api::Fail::Unavailable),
        }
        match self.queues.enable() {
            Ok(()) => {
                self.mode = Engine::On;
                Ok(())
            }
            Err(_) => {
                self.mode = if self.queues_raised() {
                    Engine::Raising
                } else {
                    Engine::Off
                };
                Err(api::Fail::Unavailable)
            }
        }
    }

    fn queues_raised(&self) -> bool {
        let (commands, faults) = self.queues.status();
        [commands, faults].into_iter().any(|csr| {
            csr & (regs::QUEUE_ENABLE
                | regs::QUEUE_ON
                | regs::QUEUE_BUSY
                | regs::QUEUE_MEM_FAULT)
                != 0
        })
    }

    /// 另一张仍在用共享队列的映射：待 retire 与待 Quiesce 都算，不能只看 programmed。
    fn other_engine_users(&self, skip: usize) -> bool {
        self.domains.iter().enumerate().any(|(index, domain)| {
            index != skip
                && domain.binding.as_ref().is_some_and(|binding| {
                    matches!(
                        binding.phase,
                        Phase::Publishing
                            | Phase::PublishFailed
                            | Phase::Published
                            | Phase::Invalidating
                            | Phase::Invalidated
                            | Phase::Quiescing
                            | Phase::Quiet
                            | Phase::Releasing
                    )
                })
        })
    }

    fn lower_engine(&mut self) -> Result<::schedule::Progress, api::Fail> {
        let handshake = match self.mode {
            Engine::Off => {
                self.verify_off()?;
                return Ok(::schedule::Progress::Done);
            }
            Engine::Lowering { handshake } => handshake,
            Engine::On | Engine::Raising => {
                let _ = self.queues.lower(regs::CQCSR);
                let _ = self.queues.lower(regs::FQCSR);
                let handshake = Handshake::new(ENGINE_NS);
                self.mode = Engine::Lowering { handshake };
                handshake
            }
        };
        let expired = handshake.expired();
        let (commands, faults) = self.queues.status();
        let requests = regs::read32(self.base, regs::PQCSR);
        let quiet = [commands, faults, requests].into_iter().all(|csr| {
            queue::quiet(csr, regs::QUEUE_ON)
                && csr & regs::QUEUE_ENABLE == 0
                && !queue::queue_error(csr)
        }) && !queue::command_error(commands);
        if !quiet {
            if expired {
                return Err(api::Fail::Unavailable);
            }
            return Ok(::schedule::Progress::Pending);
        }
        let stopped = regs::ddtp_set(self.base, regs::DDTP_MODE_OFF, None)
            .map_err(|_| api::Fail::Unavailable)?;
        if stopped & 0xF != regs::DDTP_MODE_OFF || stopped & regs::DDTP_BUSY != 0 {
            return Err(api::Fail::Unavailable);
        }
        self.verify_off()?;
        self.mode = Engine::Off;
        Ok(::schedule::Progress::Done)
    }

    fn verify_off(&self) -> Result<(), api::Fail> {
        let (commands, faults) = self.queues.status();
        let requests = regs::read32(self.base, regs::PQCSR);
        if ![commands, faults, requests].into_iter().all(|csr| {
            queue::quiet(csr, regs::QUEUE_ON)
                && csr & regs::QUEUE_ENABLE == 0
                && !queue::queue_error(csr)
        }) || queue::command_error(commands)
        {
            return Err(api::Fail::Unavailable);
        }
        let stopped = regs::read64(self.base, regs::DDTP);
        if stopped & 0xF != regs::DDTP_MODE_OFF || stopped & regs::DDTP_BUSY != 0 {
            return Err(api::Fail::Unavailable);
        }
        Ok(())
    }

    /// Host-only: put one Domain's private cursor next to exhaustion.
    #[cfg(test)]
    pub fn set_next_iova(&mut self, domain: DomainId, next: u64) -> bool {
        let Some(held) = self.domains.iter_mut().find(|held| held.id() == domain) else {
            return false;
        };
        held.next_iova = next;
        true
    }
}

//! IOMMU 那一域的核心：一次绑定的五步（取得帧引用／发布／撤销／关队列／解除）。
//!
//! 每一步都是独立效果，不合并：撤翻译不等于端点排空，关队列成功不等于资源已关。
//! 帧引用只在本记录里有一个持有方，**只在完整停链完成之后**放下；任一步没确认就
//! 保留剩余责任（引用、在途命令、本地窗）原地续办。本域不解释设备协议，也不决定
//! 何时停端点——那是协调入口（Hub）与设备恢复方的责任。

use alloc::vec::Vec;
use env::{AcquireSource, PAGE_SIZE, Permission, PieKind, PieToken, ReferenceRange, TaskId};
use execution::memory::traverse;
use resource::dock::Dock;

use crate::driver::iommu::api;
use crate::driver::iommu::binding::Binding;
use crate::driver::iommu::context;
use crate::driver::iommu::fence::Fence;
use crate::driver::iommu::hold::{
    Close as HoldClose, DeviceRange, Failure as HoldFailure, Holds, Registration, Scope,
};
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

/// 本域那一窗保留页的布局。
const DDT: usize = 0;
const ROOT: usize = 1;
const MIDDLE: usize = 2;
const LEAF: usize = 3;
const COMMANDS: usize = 4;
const FAULTS: usize = 5;
const FENCE: usize = 6;
const PAGES: usize = 7;

/// 一次取得帧引用的请求（协调入口核过之后转给核心）。
pub struct AcquireRequest {
    pub serial: u64,
    pub requester: u32,
    pub execution: TaskId,
    pub iova: u64,
    pub access: u32,
}

pub struct Core {
    /// 寄存器窗那一份映射（活到本对象最后一次访问之后）。
    _region: Dock,
    base: usize,
    /// 本实例的保留授权（声明供给交来的）。
    retention: PieToken,
    /// 本域那一窗保留页（DDT／页表／队列／完成记录）。
    pages: Pages,
    queues: CommandQueue,
    /// 帧持有账：一段保留到硬件确认停止为止（复用既有 `hold` 核心）。
    holds: Holds,
    /// 持有账要的那两步效果（本任务表上的 Acquire/Release）。
    references: References,
    /// 本次绑定；同时只服务一笔（单端点范围）。
    binding: Option<Binding>,
    /// 已结清绑定号的水位：旧号绝不能推进新绑定。
    floor: u64,
}

impl Core {
    /// 起手：取保留页、立两台队列，并**核设备目录表初态是 Off**。
    ///
    /// 起手**不编程 DDTP、不抬队列**：没有绑定时设备翻译必须不可达——
    /// 否则正常退场把这一台的页收回去时，硬件还指着一堆已回收的页。
    /// DDTP 与队列都在 `publish` 里按本次绑定抬起来，`release` 里落回去。
    pub fn start(region: Window, retention: PieToken) -> Result<Self, &'static str> {
        let dock = Dock::open(region.token).map_err(|_| "iommu: region mapping")?;
        let base = dock.view().base();
        let initial = regs::read64(base, regs::DDTP);
        if initial & 0xF != regs::DDTP_MODE_OFF || initial & regs::DDTP_BUSY != 0 {
            return Err("iommu: ddtp not off at start");
        }
        let mut pages = Pages::default();
        pages.take(retention, PAGES)?;
        let queues = CommandQueue::new(base, &pages, COMMANDS, FAULTS, FENCE);
        crate::debug::put(&alloc::format!(
            "iommu: space task={} iommu={base:#x} retain={} ddtp={initial:#x} (off)",
            env::unit::self_id().get(),
            retention.get(),
        ));
        Ok(Self {
            _region: dock,
            base,
            retention,
            pages,
            queues,
            holds: Holds::new(),
            references: References,
            binding: None,
            floor: 0,
        })
    }

    pub fn base(&self) -> usize {
        self.base
    }

    /// 此刻还持有几页帧（协调入口与场景读用）。
    pub fn held(&self) -> u32 {
        self.binding.as_ref().map_or(0, |binding| {
            if self.holds.reference(binding.range).is_some() {
                u32::try_from(binding.pages).unwrap_or(u32::MAX)
            } else {
                0
            }
        })
    }

    /// ① 取得必要帧引用：来源是执行方实际交进来的那一枚窗（内核给出的 `vestor`）。
    ///
    /// 同号同参重试复用原记录；同时只有一笔绑定；旧号一律拒。
    pub fn acquire(&mut self, request: AcquireRequest) -> Result<(), api::Fail> {
        if let Some(binding) = self.binding.as_ref() {
            if binding.serial == request.serial
                && binding.requester as u32 == request.requester
                && binding.execution == request.execution
                && binding.range.base() == request.iova
                && binding.access == request.access
            {
                return self.finish_acquire();
            }
            return Err(api::Fail::Conflict);
        }
        if request.serial <= self.floor {
            return Err(api::Fail::Conflict);
        }
        let base = dma::iova_base(request.serial).ok_or(api::Fail::Denied)?;
        if request.iova != base {
            return Err(api::Fail::Denied);
        }
        let access = env::Access::from_bits(request.access).ok_or(api::Fail::Denied)?;
        if access == env::Access::NONE {
            return Err(api::Fail::Denied);
        }
        let requester = u16::try_from(request.requester).map_err(|_| api::Fail::Denied)?;
        // 交付进来的那一枚窗：来源（内核给出的 vestor）与记号都按本次绑定核。
        // 还没交来 ⇒ `NotReady`（协调入口原样重试），不是失败。
        let source = resource::raw::pies()
            .find(|info| {
                info.alive
                    && info.kind == PieKind::Pole
                    && info.vestor == request.execution
                    && info.mark == dma::window_mark(request.serial)
                    && info.permission == (Permission::FETCH | Permission::STORE)
            })
            .map(|info| info.token)
            .ok_or(api::Fail::NotReady)?;
        // 页数由这一窗自己说：打开量一下，量完就关（不拿报文里的数当前提）。
        let dock = Dock::open(source).map_err(|_| api::Fail::Denied)?;
        let bytes = dock.view().size();
        dock.shut().map_err(|_| api::Fail::Denied)?;
        if bytes == 0 || !bytes.is_multiple_of(PAGE_SIZE) || bytes as u64 > dma::IOVA_SPAN {
            return Err(api::Fail::Denied);
        }
        let pages = bytes / PAGE_SIZE;
        let range = match DeviceRange::new(base, bytes as u64) {
            Some(range) => range,
            None => return Err(api::Fail::Denied),
        };
        // Allocate before taking a reference; after register, Binding owns every partial result.
        let mut physical = Vec::new();
        physical
            .try_reserve_exact(pages)
            .map_err(|_| api::Fail::Unavailable)?;
        physical.resize(pages, 0);
        // 持有账：先备下这一段（带本域停机凭据），再取帧引用；
        // 失败就把这一段从账上退掉，不留半个记录。
        if self
            .holds
            .attach(Scope::new(
                range,
                request.execution,
                Some(alloc::boxed::Box::new(Settled::new(
                    range,
                    self.base,
                    self.pages.at(DDT, 0) as *const u64,
                    requester,
                ))),
            ))
            .is_err()
        {
            return Err(api::Fail::Conflict);
        }
        let id = match self.holds.register(
            Registration {
                range,
                source: AcquireSource::Source {
                    token: source,
                    offset: 0,
                    count: pages,
                },
                retention: self.retention,
            },
            &mut self.references,
        ) {
            Ok(id) => id,
            Err(failure) => {
                // 没取到引用：这一段从账上退掉，不留孤儿 scope。
                let _ = failure;
                let _ = self.holds.forget(range);
                return Err(api::Fail::Denied);
            }
        };
        self.binding = Some(Binding {
            serial: request.serial,
            hold: id,
            requester,
            execution: request.execution,
            range,
            pages,
            access: request.access,
            source,
            physical,
            programmed: false,
            queued: false,
            phase: Phase::Acquiring,
            failure: None,
        });
        self.finish_acquire()
    }

    fn finish_acquire(&mut self) -> Result<(), api::Fail> {
        let binding = self.binding.as_mut().ok_or(api::Fail::Unknown)?;
        if binding.phase != Phase::Acquiring {
            return if binding.phase == Phase::Acquired {
                Ok(())
            } else {
                Err(api::Fail::Conflict)
            };
        }
        let handle = self
            .holds
            .reference(binding.range)
            .ok_or(api::Fail::Unavailable)?;
        let window = ReferenceRange::new(handle, 0, binding.pages).ok_or(api::Fail::Unavailable)?;
        traverse(self.retention, window, &mut binding.physical)
            .map_err(|_| api::Fail::Unavailable)?;
        binding.phase = Phase::Acquired;
        crate::debug::put(&alloc::format!(
            "iommu: binding {} acquired requester={} pages={} iova={:#x}",
            binding.serial,
            binding.requester,
            binding.pages,
            binding.range.base(),
        ));
        Ok(())
    }

    /// ② 发布：先记“可能可达”，再抬本轮队列、写目录项与 Sv39 叶项，
    /// 再失效＋`IOFENCE.C` 等本次完成记录。部分失败仍归本绑定的停止责任。
    pub fn publish(&mut self, serial: u64) -> Result<::schedule::Progress, api::Fail> {
        let Self {
            binding,
            queues,
            pages,
            base,
            holds,
            ..
        } = self;
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        if binding.serial != serial {
            return Err(api::Fail::Unknown);
        }
        match binding.phase {
            Phase::Acquired => binding.phase = Phase::Publishing,
            Phase::Publishing => {}
            Phase::Published => return Ok(::schedule::Progress::Done),
            _ => return Err(api::Fail::Conflict),
        }
        if !binding.queued {
            queues.enable().map_err(|_| {
                binding.failure = Some((api::PARTY, api::PUBLISH));
                api::Fail::Unavailable
            })?;
            binding.queued = true;
        }
        if !binding.programmed {
            // 先记“可能可达”（持有账那一侧），再写硬件：部分失败仍归本绑定的停止责任。
            if holds.publish_begin(binding.hold).is_err() {
                binding.failure = Some((api::PARTY, api::PUBLISH));
                return Err(api::Fail::Unavailable);
            }
            binding.programmed = true;
            let hardware = (|| -> Result<(), api::Fail> {
                let seen = regs::read64(*base, regs::DDTP);
                if seen & 0xF != regs::DDTP_MODE_OFF || seen & regs::DDTP_BUSY != 0 {
                    return Err(api::Fail::Unavailable);
                }
                // No retired IOVA path may alias the leaf page reused by this binding.
                for page in [DDT, ROOT, MIDDLE, LEAF] {
                    pages.fill(page, 0);
                }
                let context = context::DeviceContext::first_stage(pages.physical(ROOT));
                for (index, word) in context.words().iter().enumerate() {
                    pages.put64(DDT, binding.requester as usize * 64 + index * 8, *word);
                }
                table::map_physical(
                    pages,
                    ROOT,
                    MIDDLE,
                    LEAF,
                    binding.range.base() as usize,
                    &binding.physical,
                )
                .map_err(|_| api::Fail::Denied)?;
                order::sync();
                regs::ddtp_set(*base, regs::DDTP_MODE_1LVL, Some(pages.physical(DDT)))
                    .map_err(|_| api::Fail::Unavailable)?;
                queues.invalidate_all();
                Ok(())
            })();
            let ended = holds.publish_end(
                binding.hold,
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
            binding.programmed = true;
        }
        match queues.fence_step() {
            Fence::Settled => {
                binding.phase = Phase::Published;
                crate::debug::put(&alloc::format!(
                    "iommu: binding {serial} published requester={} held={} ddtp=0x{:x}",
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

    /// ③ 撤销本次翻译：清目录项并回读，再 `IOTINVAL`＋`IOFENCE.C`。
    /// 只证明本步骤：**不释放帧**，也不宣称端点已排空。
    pub fn invalidate(&mut self, serial: u64) -> Result<::schedule::Progress, api::Fail> {
        if self.binding.is_none() {
            self.floor = self.floor.max(serial);
            return Ok(::schedule::Progress::Done);
        }
        let Self {
            binding,
            queues,
            pages,
            ..
        } = self;
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        if binding.serial != serial {
            return Err(api::Fail::Unknown);
        }
        match binding.phase {
            Phase::Publishing | Phase::PublishFailed | Phase::Published => {
                binding.phase = Phase::Invalidating
            }
            Phase::Invalidating => {}
            Phase::Invalidated | Phase::Quiescing | Phase::Quiet | Phase::Releasing => {
                return Ok(::schedule::Progress::Done);
            }
            Phase::Acquiring | Phase::Acquired => {
                // 从来没发布过：没有翻译可撤，直接进“已撤”。
                binding.phase = Phase::Invalidated;
                return Ok(::schedule::Progress::Done);
            }
            _ => return Err(api::Fail::Conflict),
        }
        if binding.programmed {
            let at = binding.requester as usize * 64;
            for index in 0..8 {
                pages.put64(DDT, at + index * 8, 0);
            }
            let clean = (0..8).all(|index| pages.get64(DDT, at + index * 8) == 0);
            if !clean {
                binding.failure = Some((api::PARTY, api::INVALIDATE));
                return Err(api::Fail::Unavailable);
            }
            order::sync();
            queues.invalidate_all();
            binding.programmed = false;
        }
        match queues.fence_step() {
            Fence::Settled => {
                binding.phase = Phase::Invalidated;
                crate::debug::put(&alloc::format!(
                    "iommu: binding {serial} invalidated frames held={} (not released)",
                    binding.pages,
                ));
                Ok(::schedule::Progress::Done)
            }
            Fence::Waiting => Ok(::schedule::Progress::Pending),
            Fence::Expired => {
                binding.failure = Some((api::PARTY, api::INVALIDATE));
                Err(api::Fail::Unavailable)
            }
        }
    }

    /// ④ 关本轮队列并回读：端点停止之后才做（协调顺序在 Hub 那一侧）。
    pub fn quiesce(&mut self, serial: u64) -> Result<::schedule::Progress, api::Fail> {
        if self.binding.is_none() && serial <= self.floor {
            return Ok(::schedule::Progress::Done);
        }
        let Self {
            binding,
            queues,
            base,
            ..
        } = self;
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        if binding.serial != serial {
            return Err(api::Fail::Unknown);
        }
        match binding.phase {
            Phase::Invalidated => binding.phase = Phase::Quiescing,
            Phase::Quiescing => {}
            Phase::Quiet | Phase::Releasing => return Ok(::schedule::Progress::Done),
            _ => return Err(api::Fail::Conflict),
        }
        let (commands, faults) = queues.disable();
        let page_request = regs::read32(*base, regs::PQCSR);
        let commands_off = queue::quiet(commands, regs::QUEUE_ON);
        let faults_off = queue::quiet(faults, regs::QUEUE_ON);
        let page_unused = page_request & (regs::QUEUE_ENABLE | regs::QUEUE_ON) == 0;
        let errors = queue::command_error(commands)
            || queue::queue_error(faults)
            || queue::queue_error(page_request);
        if !(commands_off && faults_off && page_unused) || errors {
            binding.failure = Some((api::PARTY, api::QUIESCE));
            return Err(api::Fail::Unavailable);
        }
        binding.queued = false;
        binding.phase = Phase::Quiet;
        // 故障队列里的条数也回读：
        // 发布之后设备真的试图访问过而被拒的记录留在这里（现场证据，不是判据）。
        let (faults_recorded, _, fault_did, _) = queues.faults();
        crate::debug::put(&alloc::format!(
            "iommu: binding {serial} queues stopped cqcsr=0x{commands:x} fqcsr=0x{faults:x} pqcsr=0x{page_request:x} queue_enable={} faults={faults_recorded} last_did={fault_did}",
            (commands & regs::QUEUE_ENABLE) | (faults & regs::QUEUE_ENABLE),
        ));
        Ok(::schedule::Progress::Done)
    }

    /// ⑤ 解除本次引用与本地窗，并把设备目录表落 Off 回读（本轮的最终安全网）。
    /// 结清之后格空出来、水位抬高：设备可重新认领。`reason` 是本次责任的理由
    /// （Hub 在收场时给：`2` = Fault，其余按 Normal）。
    pub fn release(&mut self, serial: u64, reason: u8) -> Result<::schedule::Progress, api::Fail> {
        if self.binding.is_none() && serial <= self.floor {
            self.confirm_release(serial)?;
            return Ok(::schedule::Progress::Done);
        }
        let reason = if reason == hub_api::Close::Fault.code() {
            HoldClose::Fault
        } else {
            HoldClose::Normal
        };
        let progress = self.release_step(serial, reason)?;
        if progress == ::schedule::Progress::Done {
            self.confirm_release(serial)?;
            self.floor = self.floor.max(serial);
            self.binding = None;
            crate::debug::put(&alloc::format!(
                "iommu: binding {serial} settled, device free for a new claim"
            ));
        }
        Ok(progress)
    }

    fn release_step(
        &mut self,
        serial: u64,
        reason: HoldClose,
    ) -> Result<::schedule::Progress, api::Fail> {
        let Self {
            binding,
            base,
            holds,
            references,
            ..
        } = self;
        let binding = binding.as_mut().ok_or(api::Fail::Unknown)?;
        if binding.serial != serial {
            return Err(api::Fail::Unknown);
        }
        match binding.phase {
            Phase::Quiet => {
                // 账进收场；平台那一问（本域已停的回读）在 `retry` 里问。
                let _ = holds.close(binding.range, reason);
                binding.phase = Phase::Releasing;
            }
            Phase::Releasing => {}
            Phase::Released => return Ok(::schedule::Progress::Done),
            _ => return Err(api::Fail::Conflict),
        }
        // ① 推账：平台回读本域已停 → 放下本次引用（失败保留句柄，原地续办）。
        let _ = holds.retry(references);
        if holds.failure(binding.range).is_some() {
            binding.failure = Some((api::PARTY, api::RELEASE));
            return Err(api::Fail::Unavailable);
        }
        // ② 放下本域那一枚来源窗（执行方交进来的副本）；已随执行方退场消失也算结清。
        if binding.source != PieToken::NONE && resource::raw::alive(binding.source) {
            if env::pie::release(binding.source, env::ReleaseMode::Revoke).is_err() {
                binding.failure = Some((api::PARTY, api::RELEASE));
                return Err(api::Fail::Unavailable);
            }
        }
        binding.source = PieToken::NONE;
        // ③ 设备目录表落 Off 并回读：本轮到此再无设备翻译可达。
        let stopped = regs::ddtp_set(*base, regs::DDTP_MODE_OFF, None).map_err(|_| {
            binding.failure = Some((api::PARTY, api::RELEASE));
            api::Fail::Unavailable
        })?;
        if stopped & 0xF != regs::DDTP_MODE_OFF || stopped & regs::DDTP_BUSY != 0 {
            binding.failure = Some((api::PARTY, api::RELEASE));
            return Err(api::Fail::Unavailable);
        }
        if !holds.forget(binding.range) {
            return Err(api::Fail::Unavailable);
        }
        binding.phase = Phase::Released;
        Ok(::schedule::Progress::Done)
    }

    fn confirm_release(&self, serial: u64) -> Result<(), api::Fail> {
        let stopped = regs::read64(self.base, regs::DDTP);
        let (cqcsr, fqcsr) = self.queues.status();
        let pqcsr = regs::read32(self.base, regs::PQCSR);
        let enable = (cqcsr & regs::QUEUE_ENABLE) | (fqcsr & regs::QUEUE_ENABLE);
        let frames = self.held();
        if stopped & 0xF != regs::DDTP_MODE_OFF
            || stopped & regs::DDTP_BUSY != 0
            || ![cqcsr, fqcsr, pqcsr]
                .into_iter()
                .all(|csr| queue::quiet(csr, regs::QUEUE_ON) && !queue::queue_error(csr))
            || queue::command_error(cqcsr)
            || frames != 0
        {
            return Err(api::Fail::Unavailable);
        }
        crate::debug::put(&alloc::format!(
            "iommu: binding {serial} released ddtp=0x{stopped:x} cqcsr=0x{cqcsr:x} fqcsr=0x{fqcsr:x} pqcsr=0x{pqcsr:x} queue_enable={enable} frames={frames} stopped=true"
        ));
        Ok(())
    }

    /// 收场：两台队列停下、目录表落 Off，再关本域的保留页。
    /// 只在**所有绑定都已结清**之后调用（协调入口那一侧的门保证这一点）。
    pub fn shutdown(&mut self) -> Result<(), &'static str> {
        if let Some(binding) = self.binding.as_ref() {
            if binding.phase != Phase::Released {
                return Err("iommu: binding not settled");
            }
        }
        let (commands, faults) = self.queues.disable();
        if !queue::quiet(commands, regs::QUEUE_ON) || !queue::quiet(faults, regs::QUEUE_ON) {
            return Err("iommu: queues not quiet");
        }
        regs::ddtp_set(self.base, regs::DDTP_MODE_OFF, None).map_err(|_| "iommu: ddtp off")?;
        self.pages.close()?;
        crate::debug::put("iommu: pages closed");
        Ok(())
    }
}

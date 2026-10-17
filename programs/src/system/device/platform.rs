//! 一枚真实 RISC-V IOMMU 的**独立停机权威**：System 用来隔断外访存的入口。
//!
//! 停一个范围是一条**有序链**（照 IOMMU 规范那条 NOTE 落下来）：
//! ① **撤访问权**（`DDTP` 落 Off）→ ② 改动全局可见后**失效**（`IOTINVAL.VMA`）
//! → ③ `IOFENCE.C` 并等完成记录 → ④ **端点侧不再发起**（清 PCI bus-master 与
//! 按 virtio 那一手复位，各自回读确认）→ ⑤ 回读确认两台队列的 `on`/`busy`
//! 与 `DDTP` 那一格。
//!
//! 三处**要说清的边界**（哪些是本平台的事实，哪些是拿不到的保证）：
//! - `IOTINVAL` 只清 IOVA 缓存，**不撤访问权**——只失效而表还开着，新访问照旧成功；
//! - `IOFENCE.C` 的 `PR`/`PW` 本机被当成保留位忽略，且硬件即使支持也只保证
//!   “已处理过的请求”落全局序；**端点在途那一段**（已到线、未落内存的 posted write）
//!   要的是 interconnect-specific fence，本平台拿不出那个证据；
//! - ④ 的两步：清 bus-master 只证“不再有发起新请求的授权”；**复位那一步是这平台
//!   能给的端点侧排空凭据**——`status = 0` 同步走到所选后端的 `blk_drain()`
//!   并摘掉排队请求（在 BQL 下跑完，参见 `virtio::reset` 的注），故回读那两格
//!   就是本设备“无在途”的凭据。
//!   物理 PCIe 上“已到线、未落内存”那一档要 interconnect-specific fence，
//!   本平台给不出，也不在本凭据的范围里（不得拿它给物理硬件作保）。
//!
//! 三次“等”都是有界进展步骤，且**一帧只推一格**：`stop` 每次调用把当前那一步做到
//! 「完成 / 要等 / 硬失败」之一，要等就返 `Pending`、引用原样保留，下一帧接着来。
//! 期限随那一步保留（[`Handshake`]），重复调用既不重置它、也不重发在途的那条命令。
//! `synchronize` 与 `isolate` 走**同一条**停止链：一次性设备生命周期里没有比它更弱的
//! “正常”那一档，释放前的凭据不许偷弱化。
//!
//! 它不向设备服务要任何东西：命令队列、故障队列与端点配置空间都在本域手里。
//!
//! **所有权边**：它只持本域地址/页号，不持 `Pages` 也不持 `Dock`。调用方（管理者）
//! 必须让那两段映射活到本对象的**最后一次访问**之后。

use crate::driver::iommu::{
    fence::Fence,
    handshake::Handshake,
    order,
    queue::{self, CommandQueue},
    regs,
};
use crate::driver::pci::virtio;
use resource::retention::{DomainRange, Failure, Platform};

use super::endpoint_drain::EndpointDrain;
use super::step::Step;
use super::stop::Stop;

/// System 里那一枚 IOMMU 的停机权威：一段设备范围 ＋ 一枚随时可发命令的队列。
pub struct IommuPlatform {
    domain_range: DomainRange,
    /// 寄存器窗在本域的起址（已授权、已映射）。
    base: usize,
    /// 命令/故障/完成记录那三页（本域地址与真实物理页起址都在里面）。
    queue: CommandQueue,
    /// 端点侧不再发起那两个数；没有端点访问权时是 `None`（只停 IOMMU 那一侧）。
    endpoint: Option<EndpointDrain>,
    /// 停链走到哪一步。
    stage: Stop,
    /// ① 那一步的握手（写过之后留着，跨帧不重置）。
    revoke: Option<Handshake>,
    /// ⑤ 那一步的握手。
    queues: Option<Handshake>,
    /// 失败现场的诊断只报一次。
    reported: bool,
}

impl IommuPlatform {
    pub fn new(
        domain_range: DomainRange,
        queue: CommandQueue,
        endpoint: Option<EndpointDrain>,
    ) -> Self {
        let base = queue.base();
        Self {
            domain_range,
            base,
            queue,
            endpoint,
            stage: Stop::Revoke,
            revoke: None,
            queues: None,
            reported: false,
        }
    }
}

impl Platform for IommuPlatform {
    fn domain_range(&self) -> DomainRange {
        self.domain_range
    }

    /// 正常关闭：与隔离那一档走**同一条**停止链（释放前的凭据不更低）。
    fn synchronize(&mut self) -> Result<(), Failure> {
        self.stop()
    }

    /// 故障关闭：System 自己把整个范围停住并确认，不依赖那一头还活着。
    fn isolate(&mut self) -> Result<(), Failure> {
        self.stop()
    }
}

impl IommuPlatform {
    /// 推停链：能走几步走几步，要等就返 `Pending`，走完才返 `Ok(())`。
    ///
    /// 任何一步没确认都返错、引用保留重试；【`Stop`】记着走到哪，故重复调用幂等。
    fn stop(&mut self) -> Result<(), Failure> {
        loop {
            let (why, failure) = match self.step() {
                Step::Done => return Ok(()),
                Step::Continue => continue,
                Step::Waiting => (self.at(), Failure::Pending),
                Step::Failed => (self.at(), Failure::Unavailable),
            };
            if !self.reported {
                self.reported = true;
                crate::debug::put(&alloc::format!(
                    "system-dma: stop pending at {why} ({failure:?})"
                ));
            }
            return Err(failure);
        }
    }

    /// 当前那一步的名字（停链报告用）。
    fn at(&self) -> &'static str {
        match self.stage {
            Stop::Revoke => "ddtp off",
            Stop::Invalidate => "invalidate",
            Stop::Fence => "fence incomplete",
            Stop::Drain => "endpoint drain",
            Stop::Queues => "queues not quiet",
            Stop::Confirm => "queue confirm",
            Stop::Done => "done",
        }
    }

    /// 当前那一步推一格。
    fn step(&mut self) -> Step {
        match self.stage {
            Stop::Revoke => self.revoke(),
            Stop::Invalidate => self.invalidate(),
            Stop::Fence => self.fence(),
            Stop::Drain => self.drain(),
            Stop::Queues => self.queues(),
            Stop::Confirm => self.confirm(),
            Stop::Done => Step::Done,
        }
    }

    /// ① 撤设备访问授权：目录表落 Off（写只一次），之后每帧**单次读**到 busy 落。
    fn revoke(&mut self) -> Step {
        let handshake = match self.revoke {
            Some(handshake) => handshake,
            None => match regs::ddtp_start(self.base, regs::DDTP_MODE_OFF, None) {
                Ok(handshake) => {
                    self.revoke = Some(handshake);
                    handshake
                }
                Err(_) => return Step::Failed,
            },
        };
        match regs::ddtp_poll(self.base, regs::DDTP_MODE_OFF) {
            Ok(_) => {
                self.stage = Stop::Invalidate;
                Step::Continue
            }
            // busy 还挂着 = 还没停：期限内接着等，到期限就不冒充已停。
            Err((_, seen)) if seen & regs::DDTP_BUSY != 0 => {
                if handshake.expired() {
                    Step::Failed
                } else {
                    Step::Waiting
                }
            }
            // 其余回读 = 不认识的现场：保留并报不可用。
            Err(_) => Step::Failed,
        }
    }

    /// ② 改动全局可见之后才失效；队列从未立过（发布早就失败）就没有在途命令，两步都跳过。
    fn invalidate(&mut self) -> Step {
        if regs::read32(self.base, regs::CQCSR) & regs::QUEUE_ENABLE != 0 {
            order::sync();
            self.queue.invalidate_all();
            self.stage = Stop::Fence;
        } else {
            self.stage = Stop::Drain;
        }
        Step::Continue
    }

    /// ③ 等**本次**那条 `IOFENCE.C` 的完成记录（在途那一条原样留着）。
    fn fence(&mut self) -> Step {
        match self.queue.fence_step() {
            Fence::Settled => {
                self.stage = Stop::Drain;
                Step::Continue
            }
            Fence::Waiting => Step::Waiting,
            Fence::Expired => Step::Failed,
        }
    }

    /// ④ 端点侧不再发起：清 bus-master、按 virtio 那一手复位设备，任一没回读确认就保留。
    fn drain(&mut self) -> Step {
        if let Some(drain) = self.endpoint {
            if drain.endpoint.revoke_master() != 0 {
                return Step::Failed;
            }
            if !virtio::reset(drain.bar, &drain.caps) {
                return Step::Failed;
            }
        }
        self.stage = Stop::Queues;
        Step::Continue
    }

    /// ⑤ 两台立过的队列各自停（on 落了、busy 也清了）：写 0 只写一次，之后单次读到静。
    fn queues(&mut self) -> Step {
        let handshake = match self.queues {
            Some(handshake) => handshake,
            None => {
                let handshake = self.queue.lower(regs::CQCSR);
                self.queue.lower(regs::FQCSR);
                self.queues = Some(handshake);
                handshake
            }
        };
        let (commands, faults) = self.queue.status();
        if queue::quiet(commands, regs::QUEUE_ON) && queue::quiet(faults, regs::QUEUE_ON) {
            self.stage = Stop::Confirm;
            return Step::Continue;
        }
        if handshake.expired() {
            Step::Failed
        } else {
            Step::Waiting
        }
    }

    /// ⑥ 页请求队列若开着 = fail closed（本 slice 不支持 ATS/PRI，不称已停）；
    ///    两个队列的故障位也在这里回读。
    fn confirm(&mut self) -> Step {
        let (commands, faults) = self.queue.status();
        let page_request = regs::read32(self.base, regs::PQCSR);
        if page_request & (regs::QUEUE_ENABLE | regs::QUEUE_ON) != 0
            || queue::command_error(commands)
            || queue::queue_error(faults)
            || queue::queue_error(page_request)
        {
            return Step::Failed;
        }
        self.stage = Stop::Done;
        Step::Done
    }
}

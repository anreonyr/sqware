//! 一枚真实 RISC-V IOMMU 的**独立停机权威**：System 用来隔断外访存的入口。
//!
//! 停一个范围是一条**有序链**（照 IOMMU 规范那条 NOTE 落下来）：
//! ① **撤访问权**（`DDTP` 落 Off）→ ② 改动全局可见后**失效**（`IOTINVAL.VMA`）
//! → ③ `IOFENCE.C` 并等完成记录 → ④ **端点侧不再发起**（清 PCI bus-master 与
//! 按 virtio 那一手复位，各自回读确认）→ ⑤ 回读确认两台队列的 `on`/`busy`
//! 与 `DDTP` 那一格。
//!
//! 三处**不当作证明**（这台仿真的实现事实，或本题拿不到的保证）：
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
//! 三次“等”都是有界进展步骤：期限内没等到就返 `Pending`，引用原样保留、下次接着来。
//! `synchronize` 与 `isolate` 走**同一条**停止链：一次性设备生命周期里没有比它更弱的
//! “正常”那一档，释放前的凭据不许偷弱化。
//!
//! 它不向设备服务要任何东西：命令队列、故障队列与端点配置空间都在本域手里。
//!
//! **所有权边**：它只持本域地址/页号，不持 `Pages` 也不持 `Dock`。调用方（管理者）
//! 必须让那两段映射活到本对象的**最后一次访问**之后。

use crate::driver::iommu::{
    order,
    queue::{self, CommandQueue},
    regs,
};
use crate::driver::pci::ecam::{self, Ecam};
use crate::driver::pci::virtio::{self, Caps};
use crate::system::supply::retention::{DeviceRange, Failure, Platform};

/// 端点侧那一条收口要的四样：ECAM 段起址与功能号（清 bus-master），
/// 以及那一根 BAR 的本域基址与能力表（按 virtio 那一手复位并回读确认）。
#[derive(Clone, Copy)]
pub struct EndpointDrain {
    pub ecam: usize,
    pub requester: u16,
    pub bar: usize,
    pub caps: Caps,
}

/// System 里那一枚 IOMMU 的停机权威：一段设备范围 ＋ 一枚随时可发命令的队列。
pub struct IommuPlatform {
    range: DeviceRange,
    /// 寄存器窗在本域的起址（已授权、已映射）。
    base: usize,
    /// 命令/故障/完成记录那三页（本域地址与真实物理页起址都在里面）。
    queue: CommandQueue,
    /// 端点侧不再发起那两个数；没有端点访问权时是 `None`（只停 IOMMU 那一侧）。
    endpoint: Option<EndpointDrain>,
    /// 失败现场的诊断只报一次。
    reported: bool,
}

impl IommuPlatform {
    pub fn new(
        range: DeviceRange,
        queue: CommandQueue,
        endpoint: Option<EndpointDrain>,
    ) -> Self {
        let base = queue.base();
        Self {
            range,
            base,
            queue,
            endpoint,
            reported: false,
        }
    }
}

impl Platform for IommuPlatform {
    fn range(&self) -> DeviceRange {
        self.range
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
    /// 停住这个范围并给出凭据；任何一步没确认就返错、引用保留重试。
    ///
    /// 步骤可重入：队列立过才失效/排空（没立过就没有在途命令），`DDTP` Off 幂等。
    fn stop(&mut self) -> Result<(), Failure> {
        match self.steps() {
            Ok(()) => Ok(()),
            Err((why, failure)) => {
                if !self.reported {
                    self.reported = true;
                    crate::debug::put(&alloc::format!(
                        "system-dma: stop pending at {why} ({failure:?})"
                    ));
                }
                Err(failure)
            }
        }
    }

    fn steps(&mut self) -> Result<(), (&'static str, Failure)> {
        // ① 撤设备访问授权：目录表落 Off（幂等），回读确认 mode 与 busy。
        let stopped = match regs::ddtp_set(self.base, regs::DDTP_MODE_OFF, None) {
            Ok(seen) => seen,
            // 握手超时：busy 还挂着 = 还没停，可重试。
            Err((_, seen)) if seen & regs::DDTP_BUSY != 0 => {
                return Err(("ddtp busy", Failure::Pending));
            }
            // 其余回读 = 不认识的现场：保留并报不可用（不冒充已停）。
            Err((_, _)) => return Err(("ddtp handshake", Failure::Unavailable)),
        };
        if stopped & regs::DDTP_BUSY != 0 {
            return Err(("ddtp still busy", Failure::Pending));
        }
        if stopped & 0xF != regs::DDTP_MODE_OFF {
            return Err(("ddtp not off", Failure::Unavailable));
        }
        // ② 改动全局可见之后才失效；③ 用完成记录作“前序命令已完”的凭据。
        //    队列从未立过（发布早就失败）就没有在途命令，两步都跳过。
        let commands_up = regs::read32(self.base, regs::CQCSR) & regs::QUEUE_ENABLE != 0;
        if commands_up {
            order::sync();
            self.queue.invalidate_all();
            if !self.queue.fence() {
                return Err(("fence incomplete", Failure::Pending));
            }
        }
        // ④ 端点侧不再发起：先清 bus-master（回读确认），再按 virtio 那一手复位设备
        //    并回读确认。两者任一没确认就保留——不冒充已停。
        //    （这两步证的是“端点不再发起新请求”；已发出的写在途那一档见文件头。）
        if let Some(endpoint) = self.endpoint {
            let scan = Ecam::new(endpoint.ecam, endpoint.requester);
            scan.command_and(ecam::COMMAND_MASTER);
            if scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER != 0 {
                return Err(("endpoint bus-master", Failure::Unavailable));
            }
            if !virtio::reset(endpoint.bar, &endpoint.caps) {
                return Err(("endpoint reset", Failure::Unavailable));
            }
        }
        // ⑤ 两台立过的队列各自停（on 落了、busy 也清了）；页请求队列若开着 = fail
        //    closed（本 slice 不支持 ATS/PRI，不称已停）。
        let faults_up = regs::read32(self.base, regs::FQCSR) & regs::QUEUE_ENABLE != 0;
        if commands_up || faults_up {
            let (commands, faults) = self.queue.disable();
            if !queue::quiet(commands, regs::QUEUE_ON) || !queue::quiet(faults, regs::QUEUE_ON) {
                return Err(("queues not quiet", Failure::Pending));
            }
        }
        let (commands, faults) = self.queue.status();
        let page_request = regs::read32(self.base, regs::PQCSR);
        if page_request & (regs::QUEUE_ENABLE | regs::QUEUE_ON) != 0 {
            return Err(("page request queue active", Failure::Unavailable));
        }
        if queue::command_error(commands) || queue::queue_error(faults) {
            return Err(("queue error bit", Failure::Unavailable));
        }
        if queue::queue_error(page_request) {
            return Err(("page queue error bit", Failure::Unavailable));
        }
        Ok(())
    }
}

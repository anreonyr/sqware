//! Hub 要用的那几面：三个硬件域（PCI／IOMMU／设备恢复）＋ Control 那一面。
//!
//! 每一面都是“树上查回来的门牌 ＋ 开者”，一问一答由 `ipc::rpc` 铸、交、核。
//! 开者由内核给出（`opened_by`），Hub 不信报文里的自报。

use env::{PieToken, TaskId, Wait};
use system_api::operator::Path;
use system_client::control::{Face as ControlFace};
use system_client::operator::Face;

use crate::driver::pci::api::Call as PciCall;
use crate::driver::recovery::api::Call as RecoveryCall;
use crate::driver::shared::peer::Peer;

/// 三个硬件域的门在服务名下的坐标。
const PCI_ENTRY: &str = "pci/entry";
const IOMMU_ENTRY: &str = "iommu/entry";
const RECOVERY_ENTRY: &str = "recovery/entry";

pub struct Peers {
    pub pci: Peer<PciCall>,
    pub iommu: Peer<crate::driver::iommu::api::Call>,
    pub recovery: Peer<RecoveryCall>,
    /// 实例面：停止/核销执行实例（按真实 owner 核）。
    pub control: ControlFace,
    /// 登记面：按名字问 Control “这一条此刻是哪一枚真任务”（准入用）。
    pub state: ControlFace,
}

impl Peers {
    /// 树上把三台驱动那一扇门与 Control 两面都取回来。
    pub fn find(tree: &Face, wait: Wait) -> Result<Self, &'static str> {
        let pci = Peer::<PciCall>::find(tree, Path::new(PCI_ENTRY), PciCall::BACK, wait).map_err(|_| "pci")?;
        let iommu = Peer::<crate::driver::iommu::api::Call>::find(
            tree,
            Path::new(IOMMU_ENTRY),
            crate::driver::iommu::api::Call::BACK,
            wait,
        ).map_err(|_| "iommu")?;
        let recovery = Peer::<RecoveryCall>::find(
            tree,
            Path::new(RECOVERY_ENTRY),
            RecoveryCall::BACK,
            wait,
        ).map_err(|_| "recovery")?;
        let service = tree
            .resolve(system_api::namespace::SERVICE, wait)
            .map_err(|_| "hub: service namespace")?;
        let control = |name: &str| -> Result<ControlFace, &'static str> {
            let road = system_api::control::DIR
                .try_join(name)
                .ok_or("hub: control road")?;
            let token = tree
                .tile_from(service, &road, wait)
                .and_then(|tile| tile.token(wait))
                .map_err(|_| "hub: control face")?;
            ControlFace::of(token).map_err(|_| "hub: control face")
        };
        let instance = control("instance").map_err(|_| "control instance")?;
        let state = control(system_api::control::Grant::State.name()).map_err(|_| "control state")?;
        Ok(Self {
            pci,
            iommu,
            recovery,
            control: instance,
            state,
        })
    }

    /// 按名字问 Control：这一条此刻是哪一枚真任务（没登记/已死 ⇒ `None`）。
    pub fn registered(&self, name: &str, wait: Wait) -> Option<TaskId> {
        self.state
            .task(name.into(), wait)
            .ok()
            .filter(|task| task.get() != 0)
    }

    /// 三个硬件域的号（读数用）。
    pub fn hosts(&self) -> (TaskId, TaskId, TaskId) {
        (self.pci.host(), self.iommu.host(), self.recovery.host())
    }

    /// 把设备坐标推给执行方（放行业务那一手）：铸一枚孔、留写端。
    pub fn release_business(&self, execution: TaskId, serial: u64) -> Result<PieToken, &'static str> {
        ipc::session::establish::give(execution, crate::driver::shared::dma::coord_mark(serial))
            .map_err(|_| "hub: coordinate channel")
    }
}

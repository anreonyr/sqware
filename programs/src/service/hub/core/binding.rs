//! 一笔 DMA 安全绑定在 Hub 那本账上的全部事实：绑定编号、设备、执行方、恢复方，
//! 以及本次恢复走到哪一步、原始失败是什么。
//!
//! 它只记**协调进度**：不复制任何驱动器的寄存器值、队列游标或帧引用计数。

use env::{PieToken, TaskId};
use hub_api::{BindingId, BindingState, Close, DeviceId, Failure};

/// 一笔绑定从准备到结清的坐标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// 准备：PCI 落位。
    Place,
    /// 准备：恢复方取得恢复映射并把子授权交给执行方。
    Attach,
    /// 准备：IOMMU 取得帧引用（等执行方把窗交到它表里）。
    Acquire,
    /// 准备：IOMMU 发布翻译。
    Publish,
    /// 准备：PCI 开启 MASTER ⇒ 业务放行。
    Enable,
    /// 收场：整域停止执行方 CPU 控制。
    StopCpu,
    /// 收场：IOMMU 撤翻译＋IOTINVAL/IOFENCE。
    Invalidate,
    /// 收场：PCI 清 MASTER。
    Disable,
    /// 收场：设备恢复方 reset/drain。
    DeviceStop,
    /// 收场：IOMMU 关本轮队列并回读。
    Quiesce,
    /// 收场：IOMMU 解除引用与本地窗。
    Release,
    /// 收场：设备恢复方放下恢复映射。
    Detach,
    /// 收场：PCI 清除本绑定的准入，保留设备固定落位。
    Unplace,
    /// 本次责任全部结清。
    Done,
}

pub struct Binding {
    pub id: BindingId,
    pub claim: u64,
    pub device: DeviceId,
    pub execution: TaskId,
    pub sensor: PieToken,
    pub recovery: TaskId,
    /// 恢复方表里那一枚精确 BAR token（协调用，不解释）。
    pub bar: PieToken,
    pub pages: u32,
    pub caps: (u32, u32, u32, u32),
    pub state: BindingState,
    /// 本次责任的理由（Fault 可覆盖 Normal，不反向恢复）。
    pub reason: Close,
    pub step: Step,
    pub failure: Option<Failure>,
}

impl Binding {
    pub fn new(
        id: BindingId,
        claim: u64,
        device: DeviceId,
        execution: TaskId,
        sensor: PieToken,
        recovery: TaskId,
    ) -> Self {
        Self {
            id,
            claim,
            device,
            execution,
            sensor,
            recovery,
            bar: PieToken::NONE,
            pages: 0,
            caps: (0, 0, 0, 0),
            state: BindingState::Preparing,
            reason: Close::Normal,
            step: Step::Place,
            failure: None,
        }
    }

    /// 这一笔此刻还开着吗（未结清）。
    pub fn open(&self) -> bool {
        self.state != BindingState::Closed
    }

    pub fn close(&mut self, reason: Close) {
        if !self.open() {
            return;
        }
        if self.state != BindingState::Closing {
            self.state = BindingState::Closing;
            self.step = Step::StopCpu;
        }
        if reason == Close::Fault {
            self.reason = Close::Fault;
        }
    }
}

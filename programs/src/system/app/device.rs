//! System 帧上那两步设备面：开始一次（保留、发布、交付）与收口。
//!
//! 两步都**不阻塞**：测不出来就 `Pending`，下一帧接着来。没有设备时第一步直接
//! `Absent`，普通产品的行为一字不改。

use ::schedule::{Progress, ResMut};

use crate::system::app::Fault;
use crate::system::app::supplies::Supplies;
use crate::system::device::dma::Dma;
use crate::system::device::started::Started;
use crate::system::loader::Loader;

/// 开始一次产品 DMA：发现 → 保留 → 发布 → 交付 → 放开服务，**一帧推一步**。
///
/// 这一步**永远返 `Done`**：系统那一帧不因它停。管理器自己按帧推进（见 [`Dma::begin`]），
/// 在这里返 `Pending` 只会把本帧后面的 `retention.sweep` 挡住，反而让停机那一环跑不到。
/// 失败也**不**把 System 打停：报告一句，已经登记下的保留交给既有生命周期收口。
pub(crate) fn begin(
    mut dma: ResMut<Dma>,
    mut loader: ResMut<Loader>,
    mut supplies: ResMut<Supplies>,
) -> Result<Progress, Fault> {
    match dma.begin(&mut loader, &mut supplies) {
        Ok(Started::Absent) | Ok(Started::Running) => Ok(Progress::Done),
        // 还没装配/武装完：本帧到此，下一帧的 `device` 接着推。
        Ok(Started::Starting) => Ok(Progress::Done),
        Err(failure) => {
            crate::debug::put(&alloc::format!("system-dma: begin failed {failure:?}"));
            Ok(Progress::Done)
        }
    }
}

/// 收口：看到完成就收服务，确认（`synchronize`/`isolate`）之后才验“再也推不动”。
pub(crate) fn settle(
    mut dma: ResMut<Dma>,
    mut supplies: ResMut<Supplies>,
) -> Result<Progress, Fault> {
    Ok(dma.settle(&mut supplies))
}

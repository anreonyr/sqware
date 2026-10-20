//! 资源入口共用的那一手：本核当前任务按这次操作的许可取到这一枚，并判定它
//! 此刻是否还能承载这次操作。
//!
//! 四个资源域（Hole/Nole/Pole/Tole）各自直接调本域机制，只有身份与许可 look up
//! 以及「取到的这一枚还可用吗」共用。失败一律是 `PieFail`（权柄轴的状态），
//! 各域用自己的词汇显式折叠。

use env::{PieFail, PieToken};

use crate::core::work::pend::{self, Need, PieSnapshot, clear_heir};
use crate::core::work::room::conductor::current;

pub(super) fn accede(token: PieToken, need: Need) -> Result<PieSnapshot, PieFail> {
    let task = current().running_task().ok_or(PieFail::Denied)?;
    pend::accede(&task, token, need)
}

/// 取到的这一枚此刻是否还能承载这次操作。
///
/// 交出中的不算可用；遗留的 heir 关系顺手清掉，让这一枚回到可操作状态。
pub(super) fn usable(pie: &PieSnapshot) -> Result<(), PieFail> {
    pend::available(pie).map_err(|_| PieFail::HandedOver)?;
    let Some(h) = pie.heir().copied() else {
        return Ok(());
    };
    if let Some(task) = current().running_task() {
        clear_heir(&task, pie.token(), h);
    }
    Ok(())
}

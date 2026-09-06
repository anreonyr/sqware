// Revoke — 收回授与他人的副本（按 token，幂等）。
//
// 数据面原语：target.pies 按 token 定位 → 验 vestor == me → 摘除；Pole 视图
// （已 map 进 target space 的映射段）同步撤销。摘除后副本不复存在。
//
// 前置（envcall 入口保证）：
//   - target 任务存在
//   - current_id = 调用方 task id（鉴权「副本是我授出的」）

use alloc::sync::Weak;

use super::pie::{AnyPie, GateError};
use crate::work::unit::task::Task;

/// Revoke 数据面原语。
///
/// # Errors
/// - `Denied` — target 无该 token / 副本 vestor != me / target 已死
pub(crate) fn revoke(target: &Weak<Task>, token: u64, current_id: usize) -> Result<(), GateError> {
    let target = target.upgrade().ok_or(GateError::Denied)?;
    // 锁内：定位 + 归属 + 摘除 + 取 meta（不跨 space 操作——锁序纪律）。
    let meta = {
        let mut pies = target.pies.lock();
        let pos = pies
            .iter()
            .position(|p| p.token() == token)
            .ok_or(GateError::Denied)?;
        if pies[pos].vestor() != Some(current_id) {
            return Err(GateError::Denied);
        }
        match pies.remove(pos) {
            AnyPie::Hole(_) => None,
            AnyPie::Pole(p) => p.weak.upgrade(),
        }
    };
    // 锁外：Pole 视图撤销（幂等；Meta 已 seal 时 Drop 已清，无视图可撤）。
    if let Some(meta) = meta {
        let _ = crate::work::mail::pole::unmap(&meta, token);
    }
    Ok(())
}

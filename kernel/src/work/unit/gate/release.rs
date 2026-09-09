// Release — 自释：放下本 task 自己的一份门闩。
//
// 与 revoke 的分工：revoke 是**授与人**收回他人的副本（验 vestor == me）；release
// 是**持有人**放下自己的一份，不需要任何权限位。两者摘除后都对 Pole 同步撤销视图。
//
// 前置（envcall 入口保证）：token 在调用方 task 的权限表内。

use super::pie::{AnyPie, GateError};
use crate::work::unit::task::Task;

/// 自释数据面原语。
///
/// # Errors
/// - `Denied` — 本 task 表里没有该 token
pub(crate) fn release(task: &Task, token: usize) -> Result<(), GateError> {
    // 锁内：定位 + 摘除 + 取 meta（不跨 space 操作——锁序纪律，同 revoke）。
    let meta = {
        let mut pies = task.pies.lock();
        let pos = pies
            .iter()
            .position(|p| p.token() == token)
            .ok_or(GateError::Denied)?;
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

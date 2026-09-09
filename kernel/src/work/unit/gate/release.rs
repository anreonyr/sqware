// Release — 自释：放下本 task 自己的一份门闩（含其全部后代）。
//
// 与 revoke 的分工：revoke 是**授与人**收回他人的副本（验 sire 归属）；
// release 是**持有人**放下自己的一份，不需要任何权限位。
//
// 「放下这份，以及经它授出的一切」——级联归 `cull`。这条语义同时保证了
// 「父在则子在」：一枚门闩只能连同它的子树一起消失。
//
// 前置（envcall 入口保证）：
//   - token 在调用方 task 的权限表内
//   - 调用方不持任何 L3 锁

use super::cull;
use super::pie::GateError;
use super::snap::Snap;
use crate::work::unit::task::Task;

/// 自释数据面原语：定位 → 摘根 → 级联。
///
/// # Errors
/// - `Denied` — 本 task 表里没有该 token
pub(crate) fn release(
    task: &alloc::sync::Arc<Task>,
    token: usize,
    snap: &Snap,
) -> Result<usize, GateError> {
    let present = {
        let pies = task.pies.lock();
        pies.iter().any(|p| p.token() == token)
    };
    if !present {
        return Err(GateError::Denied);
    }
    Ok(cull::cull((task.clone(), token), snap))
}

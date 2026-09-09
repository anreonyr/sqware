// Revoke — 撤销授与他人的副本（含其全部后代，幂等）。
//
// 鉴权 = 「这枚的 `sire` 在我表里」：pie 只能复制、不能转移，故它等价于
// 「它是我授出的」。与 `Spawn` 的授权同一句话（能在我 heir 里查到 = 我是 sire）。
//
// 级联归 `cull`：摘根之后沿 `sire` 反查闭包，Pole 映射在无锁段撤销。
//
// 前置（envcall 入口保证）：
//   - target 任务存在
//   - 调用方不持任何 L3 锁

use alloc::sync::Weak;

use super::cull;
use super::pie::GateError;
use super::snap::Snap;
use crate::work::unit::task::Task;

/// Revoke 数据面原语：鉴权 → 摘根 → 级联。
///
/// # Errors
/// - `Denied` — target 无该 token / 该枚是原始自持 / 它的 sire 不在我表里
pub(crate) fn revoke(
    caller: &Task,
    target: &Weak<Task>,
    token: usize,
    snap: &Snap,
) -> Result<usize, GateError> {
    let target = target.upgrade().ok_or(GateError::Denied)?;
    // 锁内：定位 + 取 sire（不跨表操作——锁序纪律）。
    let sire = {
        let pies = target.pies.lock();
        let pie = pies
            .iter()
            .find(|p| p.token() == token)
            .ok_or(GateError::Denied)?;
        pie.sire().ok_or(GateError::Denied)?
    };
    // 鉴权：sire 在调用方表里 = 我是它的 sire。
    let mine = {
        let pies = caller.pies.lock();
        pies.iter().any(|p| p.token() == sire)
    };
    if !mine {
        return Err(GateError::Denied);
    }
    Ok(cull::cull((target, token), snap))
}

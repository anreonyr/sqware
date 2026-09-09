// Accord — 转授子集给其他 Task（原 vest）。
//
// 纯数据面原语：src 的 permission 不变；新 pie 的 permission = subset、vestor = Some(self)。
// 鉴权（VEST|BACK 权、subset 合法、target 存在）在 envcall::Accord 入口做完。
//
// 前置（envcall 入口保证）：
//   - src.permission 含 VEST 或 BACK
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//   - target 任务存在（Weak::upgrade 成功）

use alloc::sync::Weak;

use super::pie::{AnyPie, GateError, Permission, new_pie};
use crate::work::unit::task::Task;

/// Accord 数据面原语：Arc clone + 造新 Pie + push 到 target.pies。返新 token（撤销句柄）。
///
/// `target: &Weak<Task>` 避免 envcall 路径长寿命持有 `Arc<Task>`；内部短暂升级为
/// Arc，仅持锁 push 期间。
///
/// # Errors
/// - `Denied` — Weak 升级失败（target 已死 / id 不存在）
/// - `Dead` — src pie 的 Meta 已 seal
pub(crate) fn accord(
    src: &AnyPie,
    target: &Weak<Task>,
    subset: Permission,
    current_id: usize,
) -> Result<usize, GateError> {
    let target = target.upgrade().ok_or(GateError::Denied)?;
    let resource = src.resource();
    let vestor = Some(current_id);
    let granted = match src {
        AnyPie::Hole(p) => {
            let arc = p.weak.upgrade().ok_or(GateError::Dead)?;
            AnyPie::Hole(new_pie(
                resource,
                subset,
                vestor,
                alloc::sync::Arc::downgrade(&arc),
            ))
        }
        AnyPie::Pole(p) => {
            let arc = p.weak.upgrade().ok_or(GateError::Dead)?;
            AnyPie::Pole(new_pie(
                resource,
                subset,
                vestor,
                alloc::sync::Arc::downgrade(&arc),
            ))
        }
    };
    let token = granted.token();
    let mut pies = target.pies.lock();
    pies.push(granted);
    Ok(token)
}

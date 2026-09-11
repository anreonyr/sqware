// Accord — 转授子集给其他 Task（原 vest）。
//
// 纯数据面原语：src 的 permission 不变；新 pie 的 permission = subset、
// **sire = Some(src.token())**（派生边只写在这里，不碰 src 所在的表）。
// 鉴权（VEST|BACK 权、subset 合法、target 存在、BACK 守门）在 envcall::Accord
// 入口做完。
//
// 前置（envcall 入口保证）：
//   - src.permission 含 VEST 或 BACK
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//   - target 任务存在（Weak::upgrade 成功）
//   - snap::vestable(src, dst, snap) 通过（带 BACK 时）

use alloc::sync::Weak;

use super::pie::{AnyPie, GateError, Permission, new_pie};
use crate::work::unit::task::Task;

/// Accord 数据面原语：克隆资源实体的强引用 + 造新 Pie（sire 指 src）+ push 到
/// target.pies。返新 token（撤销句柄）。
///
/// `target: &Weak<Task>` 避免 envcall 路径长寿命持有 `Arc<Task>`；内部短暂升级为
/// Arc，仅持锁 push 期间。
///
/// # Errors
/// - `Denied` — Weak 升级失败（target 已死 / id 不存在）
pub(crate) fn accord(
    src: &AnyPie,
    target: &Weak<Task>,
    subset: Permission,
) -> Result<usize, GateError> {
    let target = target.upgrade().ok_or(GateError::Denied)?;
    let sire = Some(src.token());
    // 派生 = 复制资源实体的强引用（资源寿命随之延长一份）。
    let granted = match src {
        AnyPie::Hole(p) => AnyPie::Hole(new_pie(p.meta().clone(), subset, sire)),
        AnyPie::Pole(p) => AnyPie::Pole(new_pie(p.meta().clone(), subset, sire)),
        AnyPie::Void(p) => AnyPie::Void(new_pie(p.meta().clone(), subset, sire)),
    };
    let token = granted.token();
    let mut pies = target.pies.lock();
    pies.push(granted);
    Ok(token)
}

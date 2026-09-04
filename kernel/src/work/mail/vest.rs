// Vest — 把 pie 复制给其他 Task。
//
// 纯数据面原语：仅做 Arc clone + 造新 Pie + push 到 target.pies。鉴权
// （VEST 权、subset 合法、target 存在）在 envcall::Vest 入口做完。
//
// Model A：源 pie 的 permission 不变；新 pie 的 permission = subset。
//
// 前置（envcall 入口保证）：
//   - src.permission.contains(VEST)
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//   - target 任务存在（Weak::upgrade 成功）

use alloc::sync::{Arc, Weak};

use super::pie::{new_pie, AnyPie, MailError, Permission, Pie, ResourceKind};
use crate::work::unit::task::Task;

/// Vest 数据面原语。
///
/// `target: &Weak<Task>` 避免 envcall 路径长寿命持有 `Arc<Task>`，跨核时与
/// scheduler transform 的 strong_count == 1 断言冲突——内部短暂升级为 Arc，仅
/// 持锁 push 期间。
///
/// `current_task_id` 写入新 pie.vestor——BACK 权限据此验"只能还给我"。
///
/// # Errors
/// - `Denied` — Weak 升级失败（target 已死 / id 不存在）
/// - `OOM` — target.pies push 失败（Vec 扩容耗尽）
pub fn vest(
    src: &AnyPie,
    target: &Weak<Task>,
    subset: Permission,
    current_task_id: usize,
) -> Result<usize, MailError> {
    let target = target.upgrade().ok_or(MailError::Denied)?;
    let resource = src.resource();
    let vestor = Some(current_task_id);
    let new_pie = match src {
        AnyPie::Hole(_) => {
            // clone Arc<HoleMeta> → 派生 Weak → 造新 Pie<Hole>
            let arc = match src {
                AnyPie::Hole(p) => p.weak.upgrade().ok_or(MailError::Dead)?,
                _ => unreachable!(),
            };
            AnyPie::Hole(make_pie::<pie::Hole>(resource, subset, vestor, &arc))
        }
        AnyPie::Pole(_) => {
            let arc = match src {
                AnyPie::Pole(p) => p.weak.upgrade().ok_or(MailError::Dead)?,
                _ => unreachable!(),
            };
            AnyPie::Pole(make_pie::<pie::Pole>(resource, subset, vestor, &arc))
        }
    };
    let mut pies = target.pies.lock();
    pies.push(new_pie);
    Ok(pies.len() - 1)
}

/// 内部辅助：造 typed Pie（避模板重复）。
fn make_pie<T: ResourceKind>(
    resource: super::resource_table::ResourceId,
    permission: Permission,
    vestor: Option<usize>,
    arc: &Arc<T::Meta>,
) -> Pie<T> {
    new_pie::<T>(resource, permission, vestor, alloc::sync::Arc::downgrade(arc))
}

// 让 PieKind 在本模块内可用（type 拼写给 src kind 校验用）。
use super::pie as pie;
use super::resource_table;
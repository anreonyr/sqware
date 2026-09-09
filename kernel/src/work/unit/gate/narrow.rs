// Narrow — 收窄本 pie 权限（就地改写，单调）。
//
// 纯数据面原语：只做 `pie.permission` 单调收紧。sire/token/meta 不动。
// Pole 页表同步降权不在本模块：归 envcall::Narrow 适配层（先降权成功才改写）。
//
// 前置（envcall 入口保证）：
//   - src 存在、资源未封印
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//
// # Errors
// - `Denied` — 空子集 / 非单调
// - `Dead`   — 资源已封印

use super::pie::{AnyPie, GateError, Permission, Pie};

/// 就地改写一张 pie 的权限为 `subset`（含单调校验）。`alive` 由调用方按 variant
/// 取（`Pie<M>` 是泛型，不认识具体 Meta 的 `alive`）。
fn set_perm<M>(pie: &mut Pie<M>, subset: Permission, alive: bool) -> Result<(), GateError> {
    if !alive {
        return Err(GateError::Dead);
    }
    if subset.is_empty() || (subset & pie.permission) != subset {
        return Err(GateError::Denied);
    }
    pie.permission = subset;
    Ok(())
}

/// Narrow 数据面原语：按 variant 分派改写。
pub(crate) fn narrow(src: &mut AnyPie, subset: Permission) -> Result<(), GateError> {
    match src {
        AnyPie::Hole(p) => {
            let alive = p.meta().alive();
            set_perm(p, subset, alive)
        }
        AnyPie::Pole(p) => {
            let alive = p.meta().alive();
            set_perm(p, subset, alive)
        }
    }
}

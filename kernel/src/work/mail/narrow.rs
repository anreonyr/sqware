// Narrow — 收窄本 pie 权限（就地改写，单调）。
//
// 纯数据面原语：只做 `pie.permission` 单调收紧。resource/vestor/token 不动。
// Pole 页表同步降权不在本模块：归 envcall::Narrow 适配层（先降权成功才改写）。
//
// 前置（envcall 入口保证）：
//   - src 存在、alive
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//
// # Errors
// - `Denied` — 空子集 / 非单调
// - `Dead`   — pie 已死

use super::pie::{AnyPie, MailError, Permission, Pie};

/// 就地改写一张 pie 的权限为 `subset`（含单调校验）。
fn set_perm<M>(pie: &mut Pie<M>, subset: Permission) -> Result<(), MailError> {
    if !pie.alive() {
        return Err(MailError::Dead);
    }
    if subset.is_empty() || (subset & pie.permission) != subset {
        return Err(MailError::Denied);
    }
    pie.permission = subset;
    Ok(())
}

/// Narrow 数据面原语：按 variant 分派改写。
pub(crate) fn narrow(src: &mut AnyPie, subset: Permission) -> Result<(), MailError> {
    match src {
        AnyPie::Hole(p) => set_perm(p, subset),
        AnyPie::Pole(p) => set_perm(p, subset),
    }
}

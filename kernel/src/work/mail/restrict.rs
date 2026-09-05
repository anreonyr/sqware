// Restrict — 收窄本 pie 权限（就地改写，单调）。
//
// 纯数据面原语：只做 `pie.permission` 单调收紧（subset ⊆ 原 permission）。
// `resource` / `vestor` 不动——历史事实不抹（restrict 非 revoke，不收已 vest 给
// 他人的副本）。
//
// 与 Vest 的根本差异：Vest 造新 pie（复制），Restrict 改写既有 pie（`&mut`）。
//
// Pole 页表同步降权不在本模块：需 space + meta，跨核心边界，归 envcall::Restrict
// 适配层（先降权成功才改写，否则不半改）。
//
// 前置（envcall 入口保证）：
//   - src 存在、alive
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//
// # Errors
// - `Denied` — 空子集 / 非单调（subset 超出原 permission）
// - `Dead`   — pie 已死（Weak 升级失败）

use super::pie::{AnyPie, MailError, Permission, Pie, ResourceKind};

/// 就地改写一张 typed Pie 的权限为 `subset`（含单调校验）。
fn set_perm<T: ResourceKind>(pie: &mut Pie<T>, subset: Permission) -> Result<(), MailError> {
    if !pie.alive() {
        return Err(MailError::Dead);
    }
    if subset.is_empty() || (subset & pie.permission) != subset {
        return Err(MailError::Denied);
    }
    pie.permission = subset;
    Ok(())
}

/// Restrict 数据面原语：按 kind 分派改写。
pub(crate) fn restrict(src: &mut AnyPie, subset: Permission) -> Result<(), MailError> {
    match src {
        AnyPie::Hole(p) => set_perm(p, subset),
        AnyPie::Pole(p) => set_perm(p, subset),
    }
}

// Narrow — 收窄本 pie 权限（就地改写，单调）。
//
// 纯数据面原语：只做 `pie.permission` 单调收紧。sire/token/meta 不动。
// Pole 页表同步降权不在本模块：归 envcall::Narrow 适配层（先降权成功才改写）。
//
// 前置（envcall 入口保证）：
//   - src 存在、资源未封印
//   - !subset.is_empty()
//   - subset ⊆ src.permission
//   - **原带 `ONLY` ⇒ subset 必须仍含 `ONLY`**：撤掉它，这枚资源就能被复制了
//     ——"只许一个使用者"当场被洗掉。**自持枚也不例外**（它是资源事实，不是"我有
//     资格交出去"那类可摘的声明）
//
// # Errors
// - `Denied` — 空子集 / 非单调 / 撤 `ONLY`
// - `Dead`   — 资源已封印

use super::pie::{AnyPie, Mail, Permission, Pie, PieType};
use env::PieFail;

/// 就地改写一张 pie 的权限为 `subset`（含单调校验）。
///
/// **死活由这一手自己问得到**：`Mail` 契约把 `alive` 给了泛型那一侧。此前它读不到，
/// 于是四个调用点各取一次、当参数递进来（那具 `alive` 参数与"泛型读不到 meta"的注释
/// 随之退场）。
fn set_perm<T: PieType>(pie: &mut Pie<T>, subset: Permission) -> Result<(), PieFail> {
    if !pie.meta.alive() {
        return Err(PieFail::Dead);
    }
    if subset.is_empty() || (subset & pie.permission) != subset {
        return Err(PieFail::Denied);
    }
    // `ONLY` 不许被洗掉，**自持枚也不例外**：它是"这枚资源只允许一个使用者"这条
    // 资源事实的落点。摘掉它，持有者就能把资源复制出去——独占就此失效。
    if pie.permission.contains(Permission::ONLY) && !subset.contains(Permission::ONLY) {
        return Err(PieFail::Denied);
    }
    pie.permission = subset;
    Ok(())
}

/// Narrow 数据面原语：按 variant 分派改写。
pub(crate) fn narrow(src: &mut AnyPie, subset: Permission) -> Result<(), PieFail> {
    match src {
        AnyPie::Hole(p) => set_perm(p, subset),
        AnyPie::Pole(p) => set_perm(p, subset),
        // Nole 同款：收窄只改权限位——它的数据面为空，故 envcall 层没有第二步
        // （Pole 要同步降页表，Hole 与 Nole 都不用）。
        AnyPie::Nole(p) => set_perm(p, subset),
        // Tole 同款：它的数据面只有一张格子表，故 envcall 层也没有第二步。
        AnyPie::Tole(p) => set_perm(p, subset),
    }
}

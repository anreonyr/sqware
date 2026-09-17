// Accord — 转授 / 交出子集给其他 Task（原 vest）。
//
// 纯数据面原语：src 的 permission 不变；新 pie 的 permission = subset、
// **sire = Some(src.token())**（派生边只写在这里）。
//
// 四道闸在本模块（调用方不必重复判）：在表内 / 存活 / 持 VEST / 覆盖子集 /
// **未被关住**。闸之所以在这里，是因为"交出"要求就地改**调用方自己表里**那一枚
// ——`find` 给的是抄件，改抄件不动表。
//
// # 先关后授（顺序是契约，不是顺手）
//
// 带 `CAGE` 的授予 = 一次交出：源枚必须在子枚入表**之前**被关住，否则两者之间留一个
// **"两边都能用"**的窗口（多核真实可见）。反过来，"没人能用"的一小段是合法状态。
// 正因为它必须先关，就必须能回滚：目标表备不出容量时把锚清掉，等于没交出过。
//
// # 锁序
//
// `caller.pies` 与 `target.pies` 同为 L3，**绝不嵌套**：三段顺序取放
// （关 → 授 → 失败回滚），且失败路径必须**先放锁再 drop 子枚**（最后一份强引用会跑
// `Meta::drop`，那是 L3 或更外层的活）。
//
// # Errors
// - `Denied` — 表内无此 token / 不持 VEST / 子集非法 / 目标不存在
// - `Caged`  — 已被关住（"至多一个 heir"，也是独占的守卫）
// - `Dead`   — 源枚的资源已封印
// - `OoM`    — 目标表备不出容量（锚已回滚）

use alloc::sync::Weak;

use super::pie::{AnyPie, GateError, Heir, Need, Permission, new_pie};
use crate::work::room::messenger::{self, WakeKey};
use crate::work::unit::task::Task;

/// 授出 / 交出：以 `caller` 表里的 `src` 为源，造一枚 `subset` 的子枚给 `dst`。
///
/// 返：子枚在**对端**的 token（撤回句柄）。
///
/// # Errors
/// 见模块头。
pub(crate) fn accord(
    caller: &Task,
    src: usize,
    dst: &Weak<Task>,
    subset: Permission,
) -> Result<usize, GateError> {
    let target = dst.upgrade().ok_or(GateError::Denied)?;
    // ① 锁内：定位 + 四道闸 + 造子枚（尚未入表）+ 先关。放开锁再做 ②。
    let granted = {
        let mut pies = caller.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|p| p.token() == src)
            .ok_or(GateError::Denied)?;
        if !pie.alive() {
            return Err(GateError::Dead);
        }
        if !pie.allows(Need::Grant) {
            return Err(GateError::Denied);
        }
        if !pie.covers(subset) {
            return Err(GateError::Denied);
        }
        // 已被关住 ⇒ 不能再交出：一枚门闩在同一时刻至多一个 heir。
        if pie.heir().is_some() {
            return Err(GateError::Caged);
        }
        // 粘性（**只约束借入枚**）：转交时子枚必带 `CAGE`——否则"交出"会在下一跳被洗掉
        // （源枚没有带 `CAGE` 的子枚 ⇒ 它当场复原，而收方那份照样能用 ⇒ 两个使用者）。
        // 自持枚上的这一位只是"我有资格交出去"，故自持枚**可以**授出不带 `CAGE` 的子集。
        if pie.borrowed() && !subset.contains(Permission::CAGE) {
            return Err(GateError::Denied);
        }
        // 派生 = 复制资源实体的强引用（资源寿命随之延长一份）。
        let granted = match &*pie {
            AnyPie::Hole(p) => AnyPie::Hole(new_pie(p.meta().clone(), subset, Some(src))),
            AnyPie::Pole(p) => AnyPie::Pole(new_pie(p.meta().clone(), subset, Some(src))),
            AnyPie::Nole(p) => AnyPie::Nole(new_pie(p.meta().clone(), subset, Some(src))),
            AnyPie::Tole(p) => AnyPie::Tole(new_pie(p.meta().clone(), subset, Some(src))),
        };
        if subset.contains(Permission::CAGE) {
            let h = Heir {
                task: target.ident.id,
                token: granted.token(),
            };
            match pie {
                AnyPie::Hole(p) => p.heir = Some(h),
                AnyPie::Pole(p) => p.heir = Some(h),
                AnyPie::Nole(p) => p.heir = Some(h),
                AnyPie::Tole(p) => p.heir = Some(h),
            }
        }
        granted
    };
    let token = granted.token();
    // ② 目标表：备容量与入表必须同锁（备出来的那一格不能被别人用掉）。
    let mut kids = target.pies.lock();
    if kids.try_reserve(1).is_err() {
        drop(kids);
        drop(granted);
        // ③ 回滚：清锚（只写本地一格，不会失败）。
        clear_heir(caller, src);
        return Err(GateError::OoM);
    }
    kids.push(granted);
    drop(kids);
    // 落表之后**在锁外**投一次信（站点表与 `Task.pies` 同为 L3，绝不嵌套）：
    // 收方若正等着"我表里落一枚"（`UnitCall::Fall`），这一刻被叫醒。
    let _ = messenger::wake(
        WakeKey::Pies {
            task: target.ident.id,
        },
        &target.life(),
    );
    Ok(token)
}

/// 清锚：把 `task` 表里 `token` 那一枚的 `heir` 写回 `None`（幂等；返"是否真清了"）。
///
/// `heir` 只有两个写点：本模块的 [`accord`]（写）与**判据**（陈旧时清）。所有
/// "释放"路径都不需要额外动作——它们只让锚指向的那一枚消失，而"消失"由判据在
/// 下一次使用时读出来。
pub(crate) fn clear_heir(task: &Task, token: usize) -> bool {
    let mut pies = task.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    match pie {
        AnyPie::Hole(p) => p.heir.take().is_some(),
        AnyPie::Pole(p) => p.heir.take().is_some(),
        AnyPie::Nole(p) => p.heir.take().is_some(),
        AnyPie::Tole(p) => p.heir.take().is_some(),
    }
}

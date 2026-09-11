// Right — **存在权**的铸造与判定。
//
// 与其它 gate 模块的分工：accord/narrow/revoke/cull 动的是"许可怎么流转"，
// 本模块动的是"**你能不能做某件事**"——不针对任何资源。
//
// 载体是 `AnyPie::Void`（`work::mail::void`）：无数据面的权柄载体。为什么必须是
// 一个**类型**而不是 `Permission` 的一位（曾一度想那么做，用户否掉）：位说"对这份
// 资源能做什么"，与资源同轴；做成位会让**任何**资源顺带携带它，于是"这是不是那枚"
// 再也答不出来。类型是身份，位不是。
//
// 第一位消费者：建域权（`UnitCall::Build`）。判据是"调用方自己表里有没有一枚活着
// 的 Void"——**按 token 在调用方表里找**，故 token 不自证，"借来的 token"不是绕过面；
// 而 Void 没有别的用途可被误用（这正是它选 `Void` 而不是"没数据的 Hole"的理由）。

use crate::work::unit::task::Task;

use super::pie::AnyPie;

/// 调用方是否持有建域权：**它自己表里**有一枚活着且是 `Void` 的门闩。
///
/// `token` 取自 `Build` 的载荷。表里没有 → false（不是"别人的 token"，是"你没有"）；
/// 已封印 → false（`alive` 判据在 `AnyPie::alive` 里，与另两者的语义一致）。
pub(crate) fn holds_build_right(task: &Task, token: usize) -> bool {
    let pies = task.pies.lock();
    pies.iter()
        .find(|p| p.token() == token)
        .is_some_and(|p| matches!(p, AnyPie::Void(_)) && p.alive())
}

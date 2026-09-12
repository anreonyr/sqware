// 健康检查 · shell — 内核原语外壳（任务 / 团队 / 空间）的**造-收闭环**。
//
// 这三类外壳都是块侧分配：`Arc<TaskIdent>` / `Arc<Task>` / `Arc<Team>` / `Arc<Space>`。
// 当年 `leak: task 1`（`strong 0 weak 1`：载荷没了、外壳还挂着）那一族说的就是它们，
// 而它们的归还者是**退场链**（`messenger::bury` 的两步簿记清理 + 段归还）与关机时的
// `rip`——不是"把句柄丢掉"。本用例把那两步清理照原样走一遍，收尾按**逐类净额**核账。
//
// 预热一轮再判：名册 / starved 队列这些**全局表**的容量在第一次 `hold` 时会一次性增长
// （那是表的容量，不是泄漏）；故第一轮只用来吃掉这件事，判的是第二轮。
#![cfg(any(debug_assertions, feature = "framework"))]

use env::Name;

use crate::memory::allocator::statistics;
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;

/// 造-收闭环（用例体；登记在 `mod.rs` 的 `test!` 块）。
pub(super) fn accept() {
    shell_round();
    let before = statistics::kinds();
    shell_round();
    let after = statistics::kinds();
    for (k, n) in after.nonzero() {
        crate::expect!(
            n == before.get(k),
            "壳的造-收闭环未闭合 —— {}: {} → {}（整表 {}）",
            k.name(),
            before.get(k),
            n,
            after
        );
    }
}

/// 一轮：造空间（含 user 段）→ 造团队 → 造未放行任务 → 照退场链的两步清理收掉。
fn shell_round() {
    // 用户段基址取 1 GiB：页对齐、在 `mode::upper()` 之下（与 `pagetable` 用例同域）。
    const USER_BASE: usize = 0x4000_0000;

    let space = SpaceBuilder::user().build().expect("shell: build space");
    // `SpaceBuilder::user()` 只做完常量侧就位；user 段由 `dynamic(base)` 装上
    // ——loader 在映像装载结束处做的就是这一步。用户栈走 `SegmentKind::Normal`，
    // 少了它 `StackWindow::claim` 会答 `NoRegion`。
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space)
        .name(Name::new("probe").expect("shell: team name"))
        .spawn();
    let task = team
        .task()
        .name("probe-task")
        .hold()
        .expect("shell: hold task");

    // 收：照 `messenger::bury` 的两步簿记清理——团队簿记摘条、名册清死条目。
    // 少了这两步，名册里那枚弱引用会把外壳一直扣着（这正是 `prune_dead` 存在的理由：
    // 实测不调它，`task` 这一类在收完之后仍是 2）。
    let released = team.release_held(&task);
    crate::expect!(released, "shell: 摘出未放行任务失败");
    team.prune_tasks(&task);
    drop(task);
    // 栈 slot 与 trap 帧：真退场链在 `bury` 尾部用 `Space::release` 归还；本用例让
    // 团队/空间析构把它们一并带回去（`drop(team)` 走的就是空间拆除那条路）。
    drop(team);
    // 名册清死条目排在**团队析构之后**：团队自己也持着这名任务的强引用（`held` 已摘、
    // 簿记已摘，但空间/团队的拆除才让最后一份落地），早于它跑就只能等下一轮才收得掉。
    prune_dead();
}

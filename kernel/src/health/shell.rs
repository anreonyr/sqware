// 健康检查 · shell — 内核原语外壳（任务 / 团队 / 空间）的**造-收闭环**。
//
// 这三类外壳都是块侧分配：`Arc<TaskIdent>` / `Arc<Task>` / `Arc<Team>` / `Arc<Space>`。
// 当年 `leak: task 1`（`strong 0 weak 1`：载荷没了、外壳还挂着）那一族说的就是它们，
// 而它们的归还者是**退场链**（`messenger::bury` 的两步簿记清理 + 段归还）与关机时的
// `rip`——不是"把句柄丢掉"。本用例把那两步清理照原样走一遍，收尾按**逐类净额**核账。
//
// 预热一轮再判：名册 / starved 队列这些**全局表**的容量在第一次 `hold` 时会一次性增长
// （那是表的容量，不是泄漏）；故第一轮只用来吃掉这件事，判的是第二轮。
//
// `oust_round` 与它配对：**带 sire** 的造-收闭环。`TeamBuilder::sire` 一给，新域当场进
// 父方 `heir`（强持有）——没有 `Oust` 就收不回来；那一轮顺带验它的前置判据（域里还有
// 未放行的线程 ⇒ 拒绝）。
#![cfg(any(debug_assertions, feature = "framework"))]

use alloc::sync::Arc;

use env::Name;

use crate::memory::allocator::statistics;
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;
use crate::work::unit::weak::{Site, TaskWeak};

/// 造-收闭环（用例体；登记在 `mod.rs` 的 `test!` 块）。
pub(super) fn accept() {
    shell_round();
    oust_round();
    let before = statistics::kinds();
    shell_round();
    oust_round();
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

/// 用户段基址取 1 GiB：页对齐、在 `mode::upper()` 之下（与 `pagetable` 用例同域）。
const USER_BASE: usize = 0x4000_0000;

/// 一轮：造空间（含 user 段）→ 造团队 → 造未放行任务 → 照退场链的两步清理收掉。
fn shell_round() {
    let space = SpaceBuilder::user().build().expect("shell: build space");
    // `SpaceBuilder::user()` 只做完常量侧就位；user 段由 `dynamic(base)` 装上
    // ——loader 在映像装载结束处做的就是这一步。用户栈走 `SegmentKind::Normal`，
    // 少了它 `StackWindow::claim` 会答 `NoRegion`。
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space)
        .name(Name::new("probe").expect("shell: team name"))
        .spawn()
        .expect("shell: spawn team");
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

/// 一轮：**带 sire** 的造-收闭环 —— 验 `Oust`（放下子域）与它的前置判据。
///
/// 父方用本用例自己造的一枚**未放行**线程充当：健康检查跑在 `spawn_root` 之前，那时还
/// 没有"当前任务"可当 sire（`boot.rs` 的既述：用例只有单核与早启动期设施）。`heir` 是
/// Task 上的字段，与那枚线程跑没跑起来无关，故这一步不需要调度器。
fn oust_round() {
    // 父域 + 充当"父亲"的那枚线程。
    let home_space = SpaceBuilder::user().build().expect("oust: build home space");
    home_space.with_flush(|inner| inner.dynamic(USER_BASE));
    let home = TeamBuilder::new(home_space)
        .name(Name::new("oust-home").expect("oust: home name"))
        .spawn()
        .expect("oust: spawn home");
    let sire = home
        .task()
        .name("oust-sire")
        .hold()
        .expect("oust: hold sire");

    // ① 空域（没产线程）：当场算"已收尾" ⇒ 放得下；放下之后表里没有这一格，重复放下答没有。
    let space = SpaceBuilder::user().build().expect("oust: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let child = TeamBuilder::new(space)
        .sire(TaskWeak::stored(Arc::downgrade(&sire), Site::Sire))
        .name(Name::new("oust-empty").expect("oust: child name"))
        .spawn()
        .expect("oust: spawn child");
    let empty_id = child.id;
    crate::expect!(sire.heir(empty_id).is_some(), "oust: 子域没进 sire.heir");
    crate::expect!(child.all_reaped(), "oust: 空域应当算已收尾");
    drop(child);
    crate::expect!(sire.oust(empty_id).is_some(), "oust: 空域应当放得下");
    crate::expect!(sire.heir(empty_id).is_none(), "oust: 放下之后还在表里");
    crate::expect!(sire.oust(empty_id).is_none(), "oust: 重复放下应当答没有");

    // ② 域里挂一枚未放行的引导线程：不算收尾 ⇒ 拒绝，且那一格不许消失；摘净之后再放才成。
    let space = SpaceBuilder::user().build().expect("oust: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let child = TeamBuilder::new(space)
        .sire(TaskWeak::stored(Arc::downgrade(&sire), Site::Sire))
        .name(Name::new("oust-held").expect("oust: child name"))
        .spawn()
        .expect("oust: spawn child");
    let held_id = child.id;
    let unborn = child
        .task()
        .name("oust-unborn")
        .hold()
        .expect("oust: hold unborn");
    crate::expect!(!child.all_reaped(), "oust: 有未放行线程的域不该算已收尾");
    // 判据不通过 ⇒ 调用点（envcall 臂）就不摘：这里照那条路的形状走一遍。
    // （`Task::oust` 本身是纯 −1，前置判据按裁决长在臂里，不在方法里。）
    if child.all_reaped() {
        let _ = sire.oust(held_id);
    }
    crate::expect!(sire.heir(held_id).is_some(), "oust: 判据没通过却把那一格摘了");
    let released = child.release_held(&unborn);
    crate::expect!(released, "oust: 摘出未放行线程失败");
    drop(unborn);
    crate::expect!(child.all_reaped(), "oust: 摘净之后应当算已收尾");
    crate::expect!(sire.oust(held_id).is_some(), "oust: 摘净之后应当放得下");
    drop(child);

    // 收父域那一轮：照 `shell_round` 的两步清理（团队簿记摘条 + 名册清死条目）。
    let released = home.release_held(&sire);
    crate::expect!(released, "oust: 摘出 sire 失败");
    home.prune_tasks(&sire);
    drop(sire);
    drop(home);
    prune_dead();
}

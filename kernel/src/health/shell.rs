#![cfg(debug_assertions)]

use alloc::sync::Arc;

use crate::memory::allocator::statistics;
use crate::work::room::scheduler::core::prune_dead;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::TeamBuilder;
use crate::work::unit::weak::{Site, TaskWeak};

pub fn accept() {
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

const USER_BASE: usize = 0x4000_0000;

fn shell_round() {
    let space = SpaceBuilder::user().build().expect("shell: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let team = TeamBuilder::new(space).spawn().expect("shell: spawn team");
    let task = team.task().hold().expect("shell: hold task");

    let released = team.release_held(&task);
    crate::expect!(released, "shell: 摘出未放行任务失败");
    team.prune_tasks(&task);
    drop(task);
    drop(team);
    prune_dead();
}

fn oust_round() {
    let home_space = SpaceBuilder::user()
        .build()
        .expect("oust: build home space");
    home_space.with_flush(|inner| inner.dynamic(USER_BASE));
    let home = TeamBuilder::new(home_space)
        .spawn()
        .expect("oust: spawn home");
    let sire = home.task().hold().expect("oust: hold sire");

    let space = SpaceBuilder::user().build().expect("oust: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let child = TeamBuilder::new(space)
        .sire(TaskWeak::stored(Arc::downgrade(&sire), Site::Sire))
        .spawn()
        .expect("oust: spawn child");
    let empty_id = child.id;
    crate::expect!(sire.heir(empty_id).is_some(), "oust: 子域没进 sire.heir");
    crate::expect!(child.all_reaped(), "oust: 空域应当算已收尾");
    drop(child);
    crate::expect!(sire.oust(empty_id).is_some(), "oust: 空域应当放得下");
    crate::expect!(sire.heir(empty_id).is_none(), "oust: 放下之后还在表里");
    crate::expect!(sire.oust(empty_id).is_none(), "oust: 重复放下应当答没有");

    let space = SpaceBuilder::user().build().expect("oust: build space");
    space.with_flush(|inner| inner.dynamic(USER_BASE));
    let child = TeamBuilder::new(space)
        .sire(TaskWeak::stored(Arc::downgrade(&sire), Site::Sire))
        .spawn()
        .expect("oust: spawn child");
    let held_id = child.id;
    let unborn = child.task().hold().expect("oust: hold unborn");
    crate::expect!(!child.all_reaped(), "oust: 有未放行线程的域不该算已收尾");
    if child.all_reaped() {
        let _ = sire.oust(held_id);
    }
    crate::expect!(
        sire.heir(held_id).is_some(),
        "oust: 判据没通过却把那一格摘了"
    );
    let released = child.release_held(&unborn);
    crate::expect!(released, "oust: 摘出未放行线程失败");
    *unborn.state.lock() = crate::work::unit::task::TaskState::Reaped { cause: env::ExitCause::Reap, reason: 0 };
    crate::work::room::conductor::exit();
    drop(unborn);
    crate::expect!(child.all_reaped(), "oust: 摘净之后应当算已收尾");
    crate::expect!(sire.oust(held_id).is_some(), "oust: 摘净之后应当放得下");
    drop(child);

    let released = home.release_held(&sire);
    crate::expect!(released, "oust: 摘出 sire 失败");
    home.prune_tasks(&sire);
    drop(sire);
    drop(home);
    prune_dead();
}

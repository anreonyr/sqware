#![cfg(debug_assertions)]

use alloc::sync::Arc;

use env::{Mark, PieToken, TaskId};

use crate::core::work::mail::Mail;
use crate::core::work::mail::nole;
use crate::core::work::pend::{self, Permission};
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::task::Task;
use crate::core::work::unit::team::{Team, TeamBuilder, exit};

fn fixture() -> (Arc<Team>, Arc<Task>) {
    let team = TeamBuilder::new(SpaceBuilder::user().build().expect("space"))
        .spawn()
        .expect("team");
    let task = team.task().hold().expect("task");
    (team, task)
}

pub fn policy() {
    let owner = TaskId::new(41);
    let policy = exit::ExitPolicy::new();
    expect!(policy.arm(owner).is_ok(), "fresh policy must arm");
    expect!(
        policy.arm(TaskId::new(42)).is_err(),
        "another owner must not arm"
    );
    expect!(
        policy.disarm(TaskId::new(42)).is_err(),
        "another owner must not disarm"
    );
    expect!(policy.trip(), "armed death must stop");
    expect!(policy.trip(), "tripped death must stop every time");
    expect!(policy.disarm(owner).is_err(), "tripped policy must stay");
    expect!(policy.arm(owner).is_err(), "tripped policy must not re-arm");

    let policy = exit::ExitPolicy::new();
    expect!(!policy.trip(), "dormant death does not stop");
    expect!(
        policy.arm(owner).is_err(),
        "death before arm forbids later arm"
    );
}

pub fn authority() {
    let (_team, caller) = fixture();
    let meta = nole::NoleMeta::mint_guard(caller.ident.id).expect("guard mint");
    let pie = pend::Pie::try_new(
        Mail::Nole(meta),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .expect("guard pie");
    let token = pie.token();
    pend::insert(&caller, pie).expect("guard insert");
    expect!(exit::authorize(&caller, token), "guard role must authorize");

    let meta = nole::NoleMeta::mint_retention(caller.ident.id).expect("retention mint");
    let pie = pend::Pie::try_new(
        Mail::Nole(meta),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .expect("retention pie");
    let retention = pie.token();
    pend::insert(&caller, pie).expect("retention insert");
    expect!(
        !exit::authorize(&caller, retention),
        "retention role is not guard authority"
    );
    expect!(
        !exit::authorize(&caller, PieToken::mint(0xdead)),
        "absent token must not authorize"
    );
}

pub fn install_requires_debark() {
    crate::entry::boot::register_runtime_hooks();
    let (team, task) = fixture();
    let pie = pend::Pie::try_new(
        Mail::Nole(nole::NoleMeta::new(task.ident.id)),
        Mark::NONE,
        Permission::FETCH,
        None,
    )
    .expect("ordinary resource");
    let token = pie.token();
    pend::insert(&task, pie).expect("ordinary resource insert");
    expect!(
        team.guard(TaskId::new(1)).is_err(),
        "Live team must not be guarded"
    );
    team.debark().expect("debark");
    *task.state.lock() = crate::core::work::unit::task::TaskState::running(
        crate::machine::hart::HartId::new(0), 1,
    );
    expect!(team.guard(TaskId::new(1)).is_err(), "Paused phase alone is not a CPU stop proof");
    *task.state.lock() = crate::core::work::unit::task::TaskState::held();
    expect!(team.guard(TaskId::new(1)).is_ok(), "Paused team may arm");
    *task.state.lock() = crate::core::work::unit::task::TaskState::running(
        crate::machine::hart::HartId::new(0), 1,
    );
    expect!(team.unguard(TaskId::new(1)).is_err(), "Running member must retain protection");
    *task.state.lock() = crate::core::work::unit::task::TaskState::held();
    expect!(
        team.unguard(TaskId::new(1)).is_ok(),
        "Paused team may disarm"
    );
    crate::core::work::room::messenger::slay(&task);
    expect!(task.reaped(), "disarmed task must reap normally");
    expect!(
        pend::locate(&task, token).is_none(),
        "disarmed task must clean its resources"
    );
}

pub fn nested_domains() {
    use crate::core::work::unit::copies::Origin;
    use crate::core::work::unit::task::TaskWeak;
    let (parent, owner) = fixture();
    let child = TeamBuilder::new(SpaceBuilder::user().build().expect("child space"))
        .sire(TaskWeak::stored(Arc::downgrade(&owner), Origin::Sire))
        .spawn().expect("child team");
    let _member = child.task().hold().expect("child task");
    parent.debark().expect("parent debark");
    child.debark().expect("child debark");
    let authority = owner.ident.id;
    child.guard(authority).expect("child guard");
    expect!(parent.guard(authority).is_err(), "parent must not overlap guarded child");
    child.unguard(authority).expect("child unguard");
    parent.guard(authority).expect("parent guard");
    expect!(child.guard(authority).is_err(), "child must not overlap guarded parent");
    parent.unguard(authority).expect("parent unguard");
}

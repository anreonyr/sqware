use super::*;
use crate::system::control::lifecycle::{Active, Execution, ExecutionProgress, Operation, OperationKey, Request};
use crate::system::control::unit::table::{Announce, Declaration, Slot, State, Table};
use crate::system::control::unit::{Control, Input, PROGRAM};
use ::schedule::Progress;

fn reset() {
    crate::reset();
}

fn info() -> PieInfo {
    PieInfo {
        token: PieToken::mint(5),
        kind: PieKind::Nole,
        permission: Permission::FETCH,
        owner: TaskId::new(1),
        vestor: TaskId::new(0),
        mark: Mark::NONE,
        alive: true,
    }
}

fn set_inspect(result: Result<PieInfo, ()>) {
    EFFECTS.with(|effects| effects.borrow_mut().inspect = Some(result));
}

fn table(name: &'static str, team: TeamId, task: TaskId) -> Table {
    let mut table = Table::new();
    table
        .register(Declaration {
            name: name.into(),
            announce: Announce::None,
            restart: crate::unit::Ending::Resident,
        })
        .unwrap();
    table
        .attach(
            name,
            Slot::Live {
                team: Some(team),
                task,
            },
        )
        .unwrap();
    table.set_state(name, State::Starting);
    table
}

fn active(name: &'static str, task: TaskId) -> Active {
    Active(Some(Operation {
        key: OperationKey {
            request: Request {
                name: name.into(),
                action: crate::system::control::lifecycle::Action::Ruin,
            },
        },
        progress: ExecutionProgress {
            execution: Execution {
                instance: None,
                task: Some(task),
                deadline: 100,
            },
            failure: None,
        },
    }))
}

fn control(mut table: Table, name: &'static str) -> Control {
    Control {
        table: {
            table.set_guard_held(name, true);
            table
        },
        inputs: vec![Input {
            name,
            program: &PROGRAM,
        }],
        pending: Vec::new(),
        guard_token: Some(PieToken::mint(9)),
    }
}

fn launch(
    table: &mut Table,
    name: &str,
    team: TeamId,
    task: TaskId,
    required: bool,
) -> Result<Progress, UnitFail> {
    if required && !table.guard_held(name) {
        match crate::system::control::unit::guard::arm(team, PieToken::mint(9))? {
            Progress::Done => {
                table.set_guard_held(name, true);
                crate::system::control::unit::guard::resume(team)?;
            }
            Progress::Pending => return Ok(Progress::Pending),
        }
    }
    env::unit::embark_task(task).map_err(|error| error.source)?;
    Ok(Progress::Done)
}

#[test]
fn guard_selection_rejects_unknown_duplicate_and_unstarted_names() {
    assert_eq!(
        guard_selection::select(&["pci"], &["pci"], |_| true),
        Ok(&["pci"][..])
    );
    assert_eq!(
        guard_selection::select(&["gpu"], &["gpu"], |_| false),
        Err(GuardSelectionError::Unknown)
    );
    assert_eq!(
        guard_selection::select(&["pci"], &[], |_| true),
        Err(GuardSelectionError::Unstarted)
    );
    assert_eq!(
        guard_selection::select(&["pci", "pci"], &["pci"], |_| true),
        Err(GuardSelectionError::Duplicate)
    );
}

#[test]
fn empty_selection_needs_no_boot_authority() {
    reset();
    assert_eq!(app::guard::authority(&[], None), Ok(None));
    assert_eq!(app::guard::authority(&["pci"], None), Err("guard authority missing"));
}

#[test]
fn guard_authority_must_be_a_live_fetch_only_nole_owned_by_system() {
    reset();
    let token = PieToken::mint(5);
    assert_eq!(
        app::guard::authority(&["pci"], Some(token)),
        Err("guard authority unavailable")
    );

    for bad in [
        PieInfo { alive: false, ..info() },
        PieInfo { kind: PieKind::Pole, ..info() },
        PieInfo { permission: Permission::FETCH | Permission::STORE, ..info() },
        PieInfo { owner: TaskId::new(2), ..info() },
    ] {
        set_inspect(Ok(bad));
        assert_eq!(app::guard::authority(&["pci"], Some(token)), Err("guard authority"));
    }

    set_inspect(Ok(info()));
    assert_eq!(app::guard::authority(&["pci"], Some(token)), Ok(Some(token)));
}

#[test]
fn unselected_domain_never_calls_guard() {
    reset();
    assert!(!crate::system::control::unit::guard::required(&["pci"], "iommu"));
    assert!(crate::system::control::unit::guard::required(&["pci"], "pci"));
    let mut table = table("pci", TeamId::new(3), TaskId::new(4));
    assert_eq!(
        launch(&mut table, "pci", TeamId::new(3), TaskId::new(4), false),
        Ok(Progress::Done)
    );
    assert_eq!(calls(), ["embark_task"]);
    assert!(!table.guard_held("pci"));
}

#[test]
fn guarded_launch_debarks_guards_resumes_team_then_starts_task() {
    reset();
    let mut table = table("pci", TeamId::new(3), TaskId::new(4));
    assert_eq!(
        launch(&mut table, "pci", TeamId::new(3), TaskId::new(4), true),
        Ok(Progress::Done)
    );
    assert_eq!(
        calls(),
        ["debark_team", "guard", "embark_team", "embark_task"]
    );
    assert!(table.guard_held("pci"));
}

#[test]
fn guard_failure_does_not_hold_the_obligation_or_unguard() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().guard_fail = Some(UnitFail::Denied));
    let mut table = table("pci", TeamId::new(3), TaskId::new(4));
    assert_eq!(
        launch(&mut table, "pci", TeamId::new(3), TaskId::new(4), true),
        Err(UnitFail::Denied)
    );
    assert!(!table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "guard"]);
}

#[test]
fn embark_failure_after_guard_keeps_the_obligation_until_release() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().embark_team_fail = Some(UnitFail::Denied));
    let mut table = table("pci", TeamId::new(3), TaskId::new(4));
    assert_eq!(
        launch(&mut table, "pci", TeamId::new(3), TaskId::new(4), true),
        Err(UnitFail::Denied)
    );
    assert!(table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "guard", "embark_team"]);

    EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
    assert_eq!(
        crate::system::control::unit::guard::release(TeamId::new(3), PieToken::mint(9)),
        Ok(Progress::Done)
    );
    table.set_guard_held("pci", false);
    assert_eq!(calls(), ["debark_team", "unguard"]);
}

#[test]
fn external_retirement_pending_stops_before_pause_unguard_or_slay() {
    reset();
    let active = RefCell::new(active("pci", TaskId::new(4)));
    let control = RefCell::new(control(table("pci", TeamId::new(3), TaskId::new(4)), "pci"));
    let settle = RefCell::new(crate::system::app::policy::Settle { settled: false });
    assert_eq!(
        crate::system::control::lifecycle::ruin::drain(
            active.borrow(),
            control.borrow(),
            settle.borrow_mut(),
        ),
        Ok(Progress::Pending)
    );
    assert!(calls().is_empty());
    assert!(control.borrow().table.guard_held("pci"));
}

#[test]
fn busy_on_hart_keeps_the_obligation_without_unguard() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().debark_fail = Some(UnitFail::Busy));
    let active = RefCell::new(active("pci", TaskId::new(4)));
    let control = RefCell::new(control(table("pci", TeamId::new(3), TaskId::new(4)), "pci"));
    assert_eq!(
        crate::system::control::lifecycle::ruin::release_guard(
            active.borrow(),
            control.borrow_mut(),
        ),
        Ok(Progress::Pending)
    );
    assert_eq!(calls(), ["debark_team"]);
    assert!(control.borrow().table.guard_held("pci"));
}

#[test]
fn unguard_failure_keeps_the_obligation_and_blocks_destroy() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().unguard_fail = Some(UnitFail::Denied));
    let active = RefCell::new(active("pci", TaskId::new(4)));
    let control = RefCell::new(control(table("pci", TeamId::new(3), TaskId::new(4)), "pci"));
    assert!(crate::system::control::lifecycle::ruin::release_guard(
        active.borrow(),
        control.borrow_mut(),
    )
    .is_err());
    assert!(control.borrow().table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert!(crate::system::control::lifecycle::ruin::run(
        active.borrow(),
        control.borrow(),
    )
    .is_err());
    assert!(!calls().contains(&"slay_team"));
}

#[test]
fn successful_release_happens_once_and_retirement_retries_do_not_repeat_it() {
    reset();
    let active = RefCell::new(active("pci", TaskId::new(4)));
    let control = RefCell::new(control(table("pci", TeamId::new(3), TaskId::new(4)), "pci"));
    assert_eq!(
        crate::system::control::lifecycle::ruin::release_guard(
            active.borrow(),
            control.borrow_mut(),
        ),
        Ok(Progress::Done)
    );
    assert!(!control.borrow().table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert_eq!(
        crate::system::control::lifecycle::ruin::release_guard(
            active.borrow(),
            control.borrow_mut(),
        ),
        Ok(Progress::Done)
    );
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert_eq!(
        crate::system::control::lifecycle::ruin::run(active.borrow(), control.borrow()),
        Ok(Progress::Done)
    );
    assert!(calls().contains(&"slay_team"));
}

#[test]
fn resuming_a_paused_domain_does_not_rearm_guard() {
    reset();
    let table = {
        let mut table = table("pci", TeamId::new(3), TaskId::new(4));
        table.set_guard_held("pci", true);
        table
    };
    assert_eq!(crate::system::control::unit::guard::resume(TeamId::new(3)), Ok(()));
    assert_eq!(calls(), ["embark_team"]);
    assert!(table.guard_held("pci"));
}

#[test]
fn held_obligation_cannot_be_replaced_or_detached() {
    reset();
    let mut table = table("pci", TeamId::new(3), TaskId::new(4));
    table.set_guard_held("pci", true);
    assert_eq!(
        table.attach("pci", Slot::Live { team: Some(TeamId::new(3)), task: TaskId::new(4) }),
        Err(crate::system::control::unit::verdict::Fail::NotReady)
    );
    assert_eq!(
        table.detach("pci"),
        Err(crate::system::control::unit::verdict::Fail::NotReady)
    );
    assert!(table.guard_held("pci"));
}

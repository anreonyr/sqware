use super::*;
use crate::system::control::lifecycle::ruin;
use crate::system::control::unit::table::{Slot, State};
use crate::system::control::unit::verdict::Fail;
use crate::system::control::unit::Control;
use std::cell::RefCell;
use ::schedule::{Progress, Resources};

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
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &[],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Ok(Progress::Done));
    assert_eq!(calls(), ["embark_task"]);
    assert!(!resources.read::<Control>().unwrap().guard_held("pci"));
}

#[test]
fn guarded_launch_debarks_guards_resumes_team_then_starts_task() {
    reset();
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &["pci"],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Ok(Progress::Done));
    assert_eq!(
        calls(),
        ["debark_team", "guard", "embark_team", "embark_task"]
    );
    let control = resources.read::<Control>().unwrap();
    assert!(control.guard_held("pci"));
    let active = resources.read::<Active>().unwrap();
    let instance = active
        .0
        .as_ref()
        .unwrap()
        .progress
        .execution
        .instance
        .as_ref()
        .unwrap();
    assert!(instance.launched);
}

#[test]
fn guard_failure_does_not_hold_the_obligation_or_start_the_task() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().guard_fail = Some(UnitFail::Denied));
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &["pci"],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Err(Fail::NotReady));
    assert_eq!(calls(), ["debark_team", "guard"]);
    let control = resources.read::<Control>().unwrap();
    assert!(!control.guard_held("pci"));
    let active = resources.read::<Active>().unwrap();
    let instance = active
        .0
        .as_ref()
        .unwrap()
        .progress
        .execution
        .instance
        .as_ref()
        .unwrap();
    assert!(!instance.launched);
}

#[test]
fn team_resume_failure_after_guard_keeps_the_obligation_until_release() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().embark_team_fail = Some(UnitFail::Denied));
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &["pci"],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Err(Fail::NotReady));
    assert_eq!(calls(), ["debark_team", "guard", "embark_team"]);
    assert!(resources.read::<Control>().unwrap().guard_held("pci"));
    assert!(
        !resources
            .read::<Active>()
            .unwrap()
            .0
            .as_ref()
            .unwrap()
            .progress
            .execution
            .instance
            .as_ref()
            .unwrap()
            .launched
    );

    reset();
    assert_eq!(
        ruin::release_guard(
            resources.read::<Active>().unwrap(),
            resources.write::<Control>().unwrap(),
        ),
        Ok(Progress::Done)
    );
    assert_eq!(calls(), ["debark_team", "unguard"]);
    assert!(!resources.read::<Control>().unwrap().guard_held("pci"));
}

#[test]
fn task_embark_failure_after_guard_keeps_the_obligation_until_release() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().embark_task_fail = Some(UnitFail::Denied));
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &["pci"],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Err(Fail::NotReady));
    assert_eq!(
        calls(),
        ["debark_team", "guard", "embark_team", "embark_task"]
    );
    assert!(resources.read::<Control>().unwrap().guard_held("pci"));

    reset();
    assert_eq!(
        ruin::release_guard(
            resources.read::<Active>().unwrap(),
            resources.write::<Control>().unwrap(),
        ),
        Ok(Progress::Done)
    );
    assert_eq!(calls(), ["debark_team", "unguard"]);
    assert!(!resources.read::<Control>().unwrap().guard_held("pci"));
}

#[test]
fn busy_debark_stays_pending_until_the_deadline_then_fails() {
    reset();
    chrono::set(0);
    EFFECTS.with(|effects| effects.borrow_mut().debark_fail = Some(UnitFail::Busy));
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            Some(Instance::new(Service::new(TaskId::new(4)))),
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_for("pci", TeamId::new(3), TaskId::new(4)),
            "pci",
            &["pci"],
            false,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Ok(Progress::Pending));
    assert_eq!(calls(), ["debark_team"]);
    assert!(!resources.read::<Control>().unwrap().guard_held("pci"));

    chrono::set(100);
    assert_eq!(run_embark(&resources), Err(Fail::NotReady));
    assert_eq!(calls(), ["debark_team", "debark_team"]);
    assert!(!resources.read::<Control>().unwrap().guard_held("pci"));
}

#[test]
fn external_retirement_pending_stops_before_pause_unguard_or_slay() {
    reset();
    let active = RefCell::new(active_for("pci", TaskId::new(4), Action::Ruin));
    let control = RefCell::new(control_for(
        table_for("pci", TeamId::new(3), TaskId::new(4)),
        "pci",
        &["pci"],
        true,
    ));
    let settle = RefCell::new(Settle { settled: false });
    assert_eq!(
        ruin::drain(
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
    let active = RefCell::new(active_for("pci", TaskId::new(4), Action::Ruin));
    let control = RefCell::new(control_for(
        table_for("pci", TeamId::new(3), TaskId::new(4)),
        "pci",
        &["pci"],
        true,
    ));
    assert_eq!(
        ruin::release_guard(active.borrow(), control.borrow_mut()),
        Ok(Progress::Pending)
    );
    assert_eq!(calls(), ["debark_team"]);
    assert!(control.borrow().table.guard_held("pci"));
}

#[test]
fn unguard_failure_keeps_the_obligation_and_blocks_destroy() {
    reset();
    EFFECTS.with(|effects| effects.borrow_mut().unguard_fail = Some(UnitFail::Denied));
    let active = RefCell::new(active_for("pci", TaskId::new(4), Action::Ruin));
    let control = RefCell::new(control_for(
        table_for("pci", TeamId::new(3), TaskId::new(4)),
        "pci",
        &["pci"],
        true,
    ));
    assert!(ruin::release_guard(active.borrow(), control.borrow_mut()).is_err());
    assert!(control.borrow().table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert!(ruin::run(active.borrow(), control.borrow()).is_err());
    assert!(!calls().contains(&"slay_team"));
}

#[test]
fn successful_release_happens_once_and_retirement_retries_do_not_repeat_it() {
    reset();
    let active = RefCell::new(active_for("pci", TaskId::new(4), Action::Ruin));
    let control = RefCell::new(control_for(
        table_for("pci", TeamId::new(3), TaskId::new(4)),
        "pci",
        &["pci"],
        true,
    ));
    assert_eq!(
        ruin::release_guard(active.borrow(), control.borrow_mut()),
        Ok(Progress::Done)
    );
    assert!(!control.borrow().table.guard_held("pci"));
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert_eq!(
        ruin::release_guard(active.borrow(), control.borrow_mut()),
        Ok(Progress::Done)
    );
    assert_eq!(calls(), ["debark_team", "unguard"]);

    assert_eq!(
        ruin::run(active.borrow(), control.borrow()),
        Ok(Progress::Done)
    );
    assert!(calls().contains(&"slay_team"));
}

#[test]
fn resuming_a_paused_domain_keeps_held_without_rearming_guard() {
    reset();
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            None,
            100,
        ))
        .unwrap();
    resources
        .insert(control_for(
            table_in("pci", TeamId::new(3), TaskId::new(4), State::Debarked),
            "pci",
            &["pci"],
            true,
        ))
        .unwrap();
    assert_eq!(run_embark(&resources), Ok(Progress::Done));
    assert_eq!(calls(), ["embark_team"]);
    assert!(resources.read::<Control>().unwrap().guard_held("pci"));
}

#[test]
fn held_obligation_cannot_be_replaced_or_detached() {
    reset();
    let mut table = table_for("pci", TeamId::new(3), TaskId::new(4));
    table.set_guard_held("pci", true);
    assert_eq!(
        table.attach(
            "pci",
            Slot::Live {
                team: Some(TeamId::new(3)),
                task: TaskId::new(4),
            },
        ),
        Err(Fail::NotReady)
    );
    assert_eq!(table.detach("pci"), Err(Fail::NotReady));
    assert!(table.guard_held("pci"));
}

#[test]
fn embark_pre_run_ready_post_completes_the_real_lifecycle() {
    reset();
    let mut control = control_for(
        table_for("pci", TeamId::new(3), TaskId::new(4)),
        "pci",
        &["pci"],
        false,
    );
    control.pending.push(pending("pci", TaskId::new(4)));
    let mut resources = Resources::new();
    resources
        .insert(active_with(
            "pci",
            TaskId::new(4),
            Action::Embark { parent: None },
            None,
            100,
        ))
        .unwrap();
    resources.insert(control).unwrap();
    resources.insert(Roster::default()).unwrap();

    assert_eq!(run_pre(&resources), Ok(Progress::Done));
    assert_eq!(calls(), ["authorize", "connect"]);
    assert_eq!(run_embark(&resources), Ok(Progress::Done));
    assert_eq!(run_ready(&resources), Ok(Progress::Done));
    assert_eq!(run_post(&resources), Ok(Progress::Done));
    assert_eq!(
        calls(),
        [
            "authorize",
            "connect",
            "debark_team",
            "guard",
            "embark_team",
            "embark_task",
            "ready",
        ]
    );

    let control = resources.read::<Control>().unwrap();
    assert!(control.guard_held("pci"));
    assert_eq!(control.table.find("pci").unwrap().state, State::Ready);
    let active = resources.read::<Active>().unwrap();
    let instance = active
        .0
        .as_ref()
        .unwrap()
        .progress
        .execution
        .instance
        .as_ref()
        .unwrap();
    assert!(instance.launched);
    assert!(instance.identity_install.is_some());
}

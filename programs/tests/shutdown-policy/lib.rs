#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate env as abi;
extern crate self as env;

pub use abi::{TaskId, TeamId, ledger};

#[path = "fixtures/control.rs"]
mod control;
#[path = "fixtures/instance.rs"]
mod instance;
#[path = "fixtures/ending.rs"]
mod ending;
#[path = "../../src/system/app/activity.rs"]
mod activity;

mod chrono {
    thread_local! {
        static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(20_000_000_000) };
    }
    pub fn clock() -> u64 {
        NOW.with(|now| now.get())
    }
    pub fn set(value: u64) {
        NOW.with(|now| now.set(value));
    }
}

mod unit {
    pub use crate::ending::Ending;
}

use system::control::unit::table::{Announce, Declaration, Slot, State, Table};

fn register(control: &mut control::Control, name: &str, task: usize) {
    control.table.register(Declaration {
        name: name.into(),
        announce: Announce::None,
        restart: unit::Ending::Transient,
    }).unwrap();
    control.table.attach(name, Slot::Live {
        task: TaskId::new(task),
        team: Some(TeamId::new(task)),
    }).unwrap();
    control.table.set_state(name, State::Ready);
}

fn count(control: &control::Control) -> usize {
    control.living_count() + control.instances().filter(|instance| instance.team.is_some()).count()
}

fn fixture(team: Option<usize>) -> control::Control {
    chrono::set(20_000_000_000);
    let mut control = control::Control {
        table: Table::new(),
        instances: vec![],
        reclaimed_at: 0,
    };
    for task in 1..=6 {
        register(&mut control, &format!("service-{task}"), task);
    }
    if let Some(team) = team {
        control.instances.push(instance::Instance { team: Some(team) });
    }
    control
}

mod system {
    pub mod app {
        pub type Fault = ();
    }
    pub mod control {
        pub mod unit {
            pub use crate::control::Control;
            pub mod table {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/system/control/unit/table.rs"));
            }
            pub mod verdict {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/system/control/unit/verdict.rs"));
            }
            mod reclamation {
                include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/system/control/unit/reclamation.rs"));
            }
        }
    }
}

fn advance(resources: &schedule::Resources<'static>) {
    let mut frame = schedule::Schedule::sequence();
    frame.system("activity", activity::activity).unwrap();
    let mut frame = frame.build().unwrap();
    frame.prepare(resources);
    assert_eq!(
        frame.advance(&mut schedule::Cursor::default(), resources),
        Ok(schedule::Progress::Done),
    );
}

fn resources(control: control::Control) -> schedule::Resources<'static> {
    let mut resources = schedule::Resources::new();
    resources.insert(control).unwrap();
    resources.insert(activity::Activity {
        quiet: 10,
        walking: false,
    }).unwrap();
    resources
}

#[test]
fn continued_liveness_does_not_refresh_idle() {
    let resources = resources(fixture(Some(9)));
    advance(&resources);
    let activity = resources.read::<activity::Activity>().unwrap();
    assert_eq!(activity.quiet, 10, "continued liveness is not progress");
    assert!(activity.walking);
}

#[test]
fn static_reclamation_is_not_hidden_by_new_dynamic_team() {
    let mut control = fixture(None);
    assert_eq!(count(&control), 6);
    control.detach_unit("service-1").unwrap();
    control.instances.push(instance::Instance { team: Some(9) });
    assert_eq!(count(&control), 6);
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, chrono::clock());
}

#[test]
fn dynamic_reclamation_is_not_hidden_by_replacement() {
    let mut control = fixture(Some(9));
    let before = count(&control);
    control.instances[0].team = Some(10);
    control.reclaimed_at = chrono::clock();
    assert_eq!(count(&control), before);
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, chrono::clock());
}

#[test]
fn additions_alone_are_not_progress() {
    let mut control = fixture(None);
    register(&mut control, "new-service", 7);
    control.instances.push(instance::Instance { team: Some(9) });
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, 10);
}

#[test]
fn initial_or_older_reclamation_time_cannot_rewind_installation_baseline() {
    let mut control = fixture(None);
    control.reclaimed_at = 4;
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, 10);
}

#[test]
fn cleanup_started_but_not_finalized_is_not_reclamation() {
    let mut control = fixture(None);
    control.table.set_state("service-1", State::Stopping);
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, 10);
}

#[test]
fn repeated_static_detach_does_not_publish_new_progress() {
    let mut control = fixture(None);
    control.detach_unit("service-1").unwrap();
    assert_eq!(control.reclaimed_at, chrono::clock());
    let first = control.reclaimed_at;
    chrono::set(first + 1_000_000_000);
    control.detach_unit("service-1").unwrap();
    control.detach_unit("unknown").unwrap();
    assert_eq!(control.reclaimed_at, first);
}

#[test]
fn reattaching_after_detach_does_not_hide_the_completed_progress() {
    let mut control = fixture(None);
    let before = count(&control);
    control.detach_unit("service-1").unwrap();
    control.table.attach("service-1", Slot::Live {
        task: TaskId::new(20),
        team: Some(TeamId::new(20)),
    }).unwrap();
    control.table.set_state("service-1", State::Ready);
    assert_eq!(count(&control), before);
    let resources = resources(control);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, chrono::clock());
}

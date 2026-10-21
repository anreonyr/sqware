#![cfg(test)]

extern crate self as env;

#[path = "fixtures/control.rs"]
mod control;
#[path = "fixtures/instance.rs"]
mod instance;
#[path = "../../src/system/app/activity.rs"]
mod activity;

mod chrono {
    pub fn clock() -> u64 {
        20_000_000_000
    }
}

mod system {
    pub mod app {
        pub type Fault = ();
    }
    pub mod control {
        pub mod unit {
            pub use crate::control::Control;
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

fn resources(team: Option<usize>, owed: usize) -> schedule::Resources<'static> {
    let mut resources = schedule::Resources::new();
    resources.insert(control::Control {
        services: 5,
        instances: vec![instance::Instance { team }],
    }).unwrap();
    resources.insert(activity::Activity {
        owed,
        quiet: 0,
        walking: false,
    }).unwrap();
    resources
}

#[test]
fn live_dynamic_team_counts_until_reclamation() {
    let resources = resources(Some(9), 6);
    advance(&resources);
    let activity = resources.read::<activity::Activity>().unwrap();
    assert_eq!(activity.owed, 6);
    assert_eq!(activity.quiet, 0, "continued liveness is not progress");
    assert!(activity.walking);
}

#[test]
fn dynamic_reclamation_resets_idle_even_when_static_count_is_unchanged() {
    let resources = resources(Some(9), 6);
    advance(&resources);
    assert_eq!(resources.read::<activity::Activity>().unwrap().quiet, 0);
    resources.write::<control::Control>().unwrap().instances[0].team = None;
    advance(&resources);
    let activity = resources.read::<activity::Activity>().unwrap();
    assert_eq!(activity.owed, 5);
    assert_eq!(activity.quiet, chrono::clock());
}

#[test]
fn initial_sample_sets_the_idle_baseline() {
    let resources = resources(Some(9), 0);
    advance(&resources);
    let activity = resources.read::<activity::Activity>().unwrap();
    assert_eq!(activity.owed, 6);
    assert_eq!(activity.quiet, chrono::clock());
}

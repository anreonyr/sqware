use crate::support::{program, programmed};
use crate::system::control::unit::{
    table::{Announce, Declaration, Slot, State, Table},
    Control,
};
use crate::unit::{Dependency, Ending, Target};

fn live(control: &mut Control, name: &str, task: usize) {
    control
        .table
        .register(Declaration {
            name: name.into(),
            announce: Announce::None,
            restart: Ending::Resident,
        })
        .unwrap();
    control
        .table
        .attach(
            name,
            Slot::Live {
                team: Some(crate::TeamId::new(task)),
                task: crate::TaskId::new(task),
            },
        )
        .unwrap();
    control.table.set_state(name, State::Ready);
}

#[test]
fn closing_stops_the_daemon_before_the_supervised_provider() {
    let daemon = programmed(
        "daemon",
        Some("supervised"),
        Some(&[Dependency::Target("supervised")]),
    );
    let rtc = program("rtc", None);
    let targets: &'static [Target] = Box::leak(
        vec![Target {
            name: "supervised",
            members: &["rtc"],
        }]
        .into_boxed_slice(),
    );
    let mut control = Control {
        table: Table::new(),
        inputs: vec![
            crate::fixtures::Input { program: daemon },
            crate::fixtures::Input { program: rtc },
        ],
        targets,
        guarded: &[],
    };
    live(&mut control, "daemon", 20);
    live(&mut control, "rtc", 21);

    assert_eq!(
        control.closing_service_names().collect::<Vec<_>>(),
        vec!["daemon"]
    );
    control.table.set_state("daemon", State::Dead);
    assert_eq!(
        control.closing_service_names().collect::<Vec<_>>(),
        vec!["rtc"]
    );
}

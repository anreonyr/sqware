use env::{Mark, PieToken, TaskId};
use system_api::control::frame::{self, PublisherAsk, Req, Wire};
use system_api::control::Grant;
use system_api::operator::{NamespaceKey, Ops};

use crate::unit::{
    operator::{Namespace, Placement, Scope},
    table::{Announce, Declaration, Slot, State},
    verdict::Fail,
    Control, Ending, Input, OperatorConfig, UnitFile,
};

const NAMESPACE: Mark = Mark::of("service");
const RESOURCE: Mark = Mark::of("driver");

fn declaration(
    key: NamespaceKey,
    placement: Placement,
    ops: Ops,
    source_mark: Option<u64>,
) -> Namespace {
    Namespace {
        key,
        placement,
        path: "driver",
        scope: Scope::Subtree,
        ops,
        permit: None,
        source_mark,
        bindings: &[],
    }
}

fn unit(name: &'static str, declarations: Vec<Namespace>) -> &'static UnitFile {
    let namespace = Box::leak(declarations.into_boxed_slice());
    Box::leak(Box::new(UnitFile {
        name,
        operator: OperatorConfig { namespace },
    }))
}

fn input(name: &'static str, declarations: Vec<Namespace>) -> Input {
    Input {
        program: unit(name, declarations),
    }
}

fn row(
    control: &mut Control,
    name: &'static str,
    task: Option<TaskId>,
    state: State,
    named: bool,
    run: bool,
) {
    control
        .table
        .register(Declaration {
            name: String::from(name),
            announce: Announce::None,
            restart: Ending::Resident,
        })
        .unwrap();
    if let Some(task) = task {
        control
            .table
            .attach(name, Slot::Live { team: None, task })
            .unwrap();
        if named {
            control.table.mark_static(task);
        }
    }
    control.table.set_state(name, state);
    control.table.set_run_requested(name, run).unwrap();
}

fn matching(key: NamespaceKey, source_mark: Option<u64>) -> Namespace {
    declaration(key, Placement::Shared, Ops::BIND, source_mark)
}

#[test]
fn duplicate_declarations_in_one_program_are_authorized_once() {
    let mut control = Control::new(vec![input(
        "driver",
        vec![
            matching(NamespaceKey::of("service"), Some(RESOURCE.get())),
            matching(NamespaceKey::of("service"), Some(RESOURCE.get())),
        ],
    )]);
    row(
        &mut control,
        "driver",
        Some(TaskId::new(7)),
        State::Ready,
        true,
        true,
    );
    assert_eq!(
        control.publisher(NAMESPACE, RESOURCE),
        Ok(TaskId::new(7))
    );
}

#[test]
fn ambiguity_across_distinct_programs_is_denied_before_liveness() {
    let mut control = Control::new(vec![
        input(
            "dead",
            vec![matching(NamespaceKey::of("service"), Some(RESOURCE.get()))],
        ),
        input(
            "live",
            vec![matching(NamespaceKey::of("service"), Some(RESOURCE.get()))],
        ),
    ]);
    row(
        &mut control,
        "dead",
        Some(TaskId::new(1)),
        State::Dead,
        true,
        true,
    );
    row(
        &mut control,
        "live",
        Some(TaskId::new(2)),
        State::Ready,
        true,
        true,
    );
    assert_eq!(control.publisher(NAMESPACE, RESOURCE), Err(Fail::Denied));
}

#[test]
fn unrelated_namespace_read_only_and_instance_declarations_are_excluded() {
    let cases = [
        matching(NamespaceKey::of("other"), Some(RESOURCE.get())),
        declaration(
            NamespaceKey::of("service"),
            Placement::Shared,
            Ops::FIND.union(Ops::INSPECT),
            Some(RESOURCE.get()),
        ),
        declaration(
            NamespaceKey::of("service"),
            Placement::Instance,
            Ops::BIND,
            Some(RESOURCE.get()),
        ),
    ];
    for declaration in cases {
        let mut control = Control::new(vec![input("driver", vec![declaration])]);
        row(
            &mut control,
            "driver",
            Some(TaskId::new(3)),
            State::Ready,
            true,
            true,
        );
        assert_eq!(control.publisher(NAMESPACE, RESOURCE), Err(Fail::Unknown));
    }
}

#[test]
fn zero_marks_are_denied() {
    let control = Control::new(Vec::new());
    assert_eq!(
        control.publisher(Mark::NONE, RESOURCE),
        Err(Fail::Denied)
    );
    assert_eq!(
        control.publisher(NAMESPACE, Mark::NONE),
        Err(Fail::Denied)
    );
}

#[test]
fn missing_row_is_unknown() {
    let control = Control::new(vec![input(
        "driver",
        vec![matching(NamespaceKey::of("service"), Some(RESOURCE.get()))],
    )]);
    assert_eq!(control.publisher(NAMESPACE, RESOURCE), Err(Fail::Unknown));
}

#[test]
fn lifecycle_mismatches_are_not_ready() {
    let cases = [
        (Some(TaskId::new(3)), State::Starting, true, true, false),
        (Some(TaskId::new(3)), State::Ready, false, true, false),
        (Some(TaskId::new(3)), State::Ready, true, false, false),
        (None, State::Ready, false, true, false),
        (Some(TaskId::new(0)), State::Ready, true, true, false),
        (Some(TaskId::new(3)), State::Ready, true, true, true),
    ];
    for (task, state, named, run, dead) in cases {
        let mut control = Control::new(vec![input(
            "driver",
            vec![matching(NamespaceKey::of("service"), Some(RESOURCE.get()))],
        )]);
        row(&mut control, "driver", task, state, named, run);
        if dead {
            if let Some(task) = task {
                control.dead.push(task);
            }
        }
        assert_eq!(
            control.publisher(NAMESPACE, RESOURCE),
            Err(Fail::NotReady)
        );
    }
}

#[test]
fn real_named_ready_publisher_returns_expected_task() {
    let mut control = Control::new(vec![input(
        "driver",
        vec![matching(NamespaceKey::of("service"), Some(RESOURCE.get()))],
    )]);
    row(
        &mut control,
        "driver",
        Some(TaskId::new(9)),
        State::Ready,
        true,
        true,
    );
    assert_eq!(
        control.publisher(NAMESPACE, RESOURCE),
        Ok(TaskId::new(9))
    );
}

#[test]
fn publisher_ask_roundtrips_truncates_and_maps_to_state_grant() {
    let back = PieToken::mint(0x0102);
    let namespace = Mark::new(0x8123_4567_89AB_CDEF);
    let mark = Mark::new(0xFEDC_BA98_7654_3210);
    let mut bytes = [0u8; frame::Ask::LEN];
    let len = Req::Publisher { namespace, mark }
        .store(back, &mut bytes)
        .unwrap();
    assert_eq!(len, PublisherAsk::LEN);
    assert!(PublisherAsk::LEN <= frame::Ask::LEN);
    assert_eq!(bytes[0], frame::PUBLISHER);
    assert_eq!(&bytes[1..9], &namespace.to_bytes());
    assert_eq!(&bytes[9..17], &mark.to_bytes());
    assert_eq!(&bytes[17..25], &back.to_bytes());
    assert_eq!(
        Wire::take(&bytes[..len]),
        Some((Some(Wire::Publisher { namespace, mark }), back))
    );
    assert_eq!(
        Grant::for_wire(&Wire::Publisher { namespace, mark }),
        Grant::State
    );
    assert!(Wire::take(&bytes[..len - 1]).is_none());
    bytes[len] = 0;
    assert!(Wire::take(&bytes[..len + 1]).is_none());
}

#[test]
fn publisher_opcode_does_not_fall_back_to_a_legacy_name() {
    let back = PieToken::mint(0x0102);
    // Little-endian mark bytes: 03 'a' 'b' 'c' would look like a 3-byte name
    // to the legacy Ask decoder.
    let namespace = Mark::new(0x6362_6103);
    let mark = Mark::new(0x11);
    let mut bytes = [0u8; frame::Ask::LEN];
    let len = Req::Publisher { namespace, mark }
        .store(back, &mut bytes)
        .unwrap();
    let decoded = Wire::take(&bytes[..len]).unwrap();
    assert_eq!(decoded.1, back);
    assert_eq!(
        decoded.0,
        Some(Wire::Publisher { namespace, mark })
    );
    assert!(!matches!(decoded.0, Some(Wire::Task(_))));
}

#[test]
fn zero_marks_are_explicit_wire_values() {
    let back = PieToken::mint(0x0102);
    let mut bytes = [0u8; frame::Ask::LEN];
    let len = Req::Publisher {
        namespace: Mark::NONE,
        mark: Mark::NONE,
    }
    .store(back, &mut bytes)
    .unwrap();
    assert_eq!(
        Wire::take(&bytes[..len]),
        Some((
            Some(Wire::Publisher {
                namespace: Mark::NONE,
                mark: Mark::NONE,
            }),
            back,
        ))
    );
}

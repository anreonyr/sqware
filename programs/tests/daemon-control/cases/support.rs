use alloc::string::String;

use crate::fixtures::Waiters;
use crate::system::control::lifecycle::{
    self, Action, Active, Execution, ExecutionProgress, Operation, OperationKey, Origin, Request,
    Tracked,
};
use crate::system::control::unit::{
    Control, Input,
    table::{Announce, Declaration, Slot, State, Table},
};
use crate::unit::{ControlConfig, Dependency, Ending, Target, UnitFile};
use schedule::{Progress, Resources};
use system_api::control::frame;

pub fn reset() {
    crate::DEAD.with(|dead| dead.borrow_mut().clear());
    crate::ANSWERS.with(|answers| answers.borrow_mut().clear());
}

pub fn program(name: &'static str, supervision: Option<&'static str>) -> &'static UnitFile {
    Box::leak(Box::new(UnitFile {
        name,
        control: ControlConfig {
            supervision,
            after: None,
        },
    }))
}

pub fn programmed(
    name: &'static str,
    supervision: Option<&'static str>,
    after: Option<&'static [Dependency]>,
) -> &'static UnitFile {
    Box::leak(Box::new(UnitFile {
        name,
        control: ControlConfig { supervision, after },
    }))
}

pub fn target(members: &'static [&'static str]) -> &'static [Target] {
    Box::leak(
        vec![Target {
            name: "scene",
            members,
        }]
        .into_boxed_slice(),
    )
}

pub fn control(marked: bool, requested: bool, state: State, slot: Slot) -> Control {
    let mut table = Table::new();
    table
        .register(Declaration {
            name: "main".into(),
            announce: Announce::None,
            restart: Ending::Resident,
        })
        .unwrap();
    table
        .attach(
            "main",
            Slot::Live {
                team: None,
                task: crate::TaskId::new(10),
            },
        )
        .unwrap();
    table.set_state("main", State::Ready);
    if marked {
        table.mark_static(crate::TaskId::new(10));
    }
    table.set_run_requested("main", requested).unwrap();
    table
        .register(Declaration {
            name: "worker".into(),
            announce: Announce::None,
            restart: Ending::Resident,
        })
        .unwrap();
    table.set_state("worker", state);
    if !matches!(slot, Slot::None) {
        table.attach("worker", slot).unwrap();
        table.set_state("worker", state);
    }
    table.set_run_requested("worker", true).unwrap();
    Control {
        table,
        inputs: vec![
            Input {
                program: program("main", Some("scene")),
            },
            Input {
                program: program("worker", None),
            },
        ],
        targets: target(&["worker"]),
        guarded: &[],
    }
}

pub fn active(action: Action, origin: Origin, name: &str) -> Active {
    Active(Some(Operation {
        key: OperationKey {
            request: Request {
                name: name.into(),
                directive: lifecycle::Directive { action, origin },
                back: None,
            },
            waiters: Waiters::with_capacity(8),
        },
        progress: ExecutionProgress {
            execution: Execution {
                instance: None,
                task: None,
                deadline: 0,
            },
            failure: None,
        },
    }))
}

pub fn admit(
    control: Control,
    active: Active,
    settle: crate::system::app::policy::Settle,
) -> Result<Progress, lifecycle::Fail> {
    let mut resources = Resources::new();
    resources.insert(control).unwrap();
    resources.insert(active).unwrap();
    resources.insert(settle).unwrap();
    lifecycle::authority::admit(
        resources.read::<Active>().unwrap(),
        resources.read::<Control>().unwrap(),
        resources
            .read::<crate::system::app::policy::Settle>()
            .unwrap(),
    )
}

pub fn recheck(
    control: Control,
    active: Active,
    settle: crate::system::app::policy::Settle,
) -> Result<Progress, lifecycle::Fail> {
    let mut resources = Resources::new();
    resources.insert(control).unwrap();
    resources.insert(active).unwrap();
    resources.insert(settle).unwrap();
    lifecycle::authority::recheck(
        resources.read::<Active>().unwrap(),
        resources.read::<Control>().unwrap(),
        resources
            .read::<crate::system::app::policy::Settle>()
            .unwrap(),
    )
}

pub fn open() -> crate::system::app::policy::Settle {
    crate::system::app::policy::Settle { closing: false }
}

pub fn sender() -> crate::rpc::reply::Sender<frame::Said> {
    crate::rpc::reply::Sender(core::marker::PhantomData)
}

pub fn tracked(action: Action, origin: Origin, task: Option<crate::TaskId>) -> Tracked {
    let mut waiters = Waiters::with_capacity(2);
    waiters.push(sender());
    tracked_with_waiters(action, origin, task, waiters)
}

pub fn tracked_with_waiters(
    action: Action,
    origin: Origin,
    task: Option<crate::TaskId>,
    waiters: Waiters,
) -> Tracked {
    Tracked {
        operation: Operation {
            key: OperationKey {
                request: Request {
                    name: String::from("worker"),
                    directive: lifecycle::Directive { action, origin },
                    back: Some(sender()),
                },
                waiters,
            },
            progress: ExecutionProgress {
                execution: Execution {
                    instance: None,
                    task,
                    deadline: 0,
                },
                failure: None,
            },
        },
        cursor: schedule::Cursor::default(),
        complete: false,
    }
}

pub fn request(action: Action, origin: Origin) -> Request {
    Request {
        name: String::from("worker"),
        directive: lifecycle::Directive { action, origin },
        back: Some(sender()),
    }
}

pub fn recovery() -> Origin {
    Origin::Recovery {
        caller: crate::TaskId::new(10),
    }
}

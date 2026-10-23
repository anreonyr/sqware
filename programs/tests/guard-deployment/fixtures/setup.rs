use crate::system::control::{
    lifecycle::{
        embark,
        Action, Active, Execution, ExecutionProgress, Instance, Operation, OperationKey, Request,
    },
    unit::{
        table::{Announce, Declaration, Slot, State, Table},
        verdict::Fail,
        Control, Input, Pending, Service,
    },
    identity::Roster,
};
use crate::{PieInfo, PieKind, PieToken, Mark, Permission, TaskId, TeamId};
use ::schedule::{Progress, Resources};

pub static PROGRAM: crate::unit::UnitFile = crate::unit::UnitFile {
    name: "pci",
    control: crate::unit::ControlConfig {
        ready: crate::unit::Ready::OnSpawn,
    },
};

pub fn info() -> PieInfo {
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

pub fn set_inspect(result: Result<PieInfo, ()>) {
    crate::EFFECTS.with(|effects| effects.borrow_mut().inspect = Some(result));
}

pub fn table_in(
    name: &'static str,
    team: TeamId,
    task: TaskId,
    state: State,
) -> Table {
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
    table.set_state(name, state);
    table
}

pub fn table_for(name: &'static str, team: TeamId, task: TaskId) -> Table {
    table_in(name, team, task, State::Starting)
}

pub fn active_with(
    name: &'static str,
    task: TaskId,
    action: Action,
    instance: Option<Instance>,
    deadline: u64,
) -> Active {
    Active(Some(Operation {
        key: OperationKey {
            request: Request {
                name: name.into(),
                action,
            },
        },
        progress: ExecutionProgress {
            execution: Execution {
                instance,
                task: Some(task),
                deadline,
            },
            failure: None,
        },
    }))
}

pub fn active_for(name: &'static str, task: TaskId, action: Action) -> Active {
    active_with(name, task, action, None, 100)
}

pub fn control_for(
    table: Table,
    name: &'static str,
    guarded: &'static [&'static str],
    held: bool,
) -> Control {
    let mut control = Control {
        table,
        inputs: vec![Input { program: &PROGRAM }],
        pending: Vec::new(),
        guarded,
        guard_token: Some(PieToken::mint(9)),
    };
    if held {
        control.hold_guard(name);
    }
    control
}

pub fn pending(name: &'static str, task: TaskId) -> Pending {
    Pending {
        name: name.into(),
        service: Service::new(task),
    }
}

pub fn run_embark(resources: &Resources<'static>) -> Result<Progress, Fail> {
    let active = resources.write::<Active>().unwrap();
    let control = resources.write::<Control>().unwrap();
    embark::run(active, control)
}

pub fn run_pre(resources: &Resources<'static>) -> Result<Progress, Fail> {
    let active = resources.write::<Active>().unwrap();
    let control = resources.write::<Control>().unwrap();
    let roster = resources.write::<Roster>().unwrap();
    embark::pre(active, control, roster)
}

pub fn run_ready(resources: &Resources<'static>) -> Result<Progress, Fail> {
    let active = resources.write::<Active>().unwrap();
    let control = resources.write::<Control>().unwrap();
    embark::ready(active, control)
}

pub fn run_post(resources: &Resources<'static>) -> Result<Progress, Fail> {
    let active = resources.read::<Active>().unwrap();
    let control = resources.read::<Control>().unwrap();
    embark::post(active, control)
}

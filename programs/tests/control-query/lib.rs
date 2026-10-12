#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as programs;
pub mod chrono {
    pub fn clock() -> u64 {
        0
    }
}
pub mod debug {
    pub fn put(_: &str) {}
}
pub use abi::{PieToken, TaskId, Wait};
use std::cell::RefCell;
thread_local! {
    static ANSWERS: RefCell<Vec<system_api::control::frame::Said>> = const { RefCell::new(Vec::new()) };
    static JOINED: RefCell<bool> = const { RefCell::new(false) };
}
pub mod unit {
    #[derive(Clone, Copy)]
    pub enum Dependency {
        Unit(&'static str),
        Target(&'static str),
        Bootstrap(()),
    }
    pub struct ControlConfig {
        pub after: Option<&'static [Dependency]>,
    }
    pub struct UnitFile {
        pub name: &'static str,
        pub control: ControlConfig,
    }
    pub struct Target {
        pub name: &'static str,
        pub members: &'static [&'static str],
    }
    pub fn self_id() -> crate::TaskId {
        crate::TaskId::new(1)
    }
    pub fn join_task(_: crate::TaskId, _: crate::Wait) -> Result<bool, ()> {
        Ok(crate::JOINED.with(|value| *value.borrow()))
    }
}
pub mod rpc {
    pub enum Fail {
        Receive(()),
        Other,
    }
    pub struct Rejected {
        pub fail: Fail,
    }
    pub mod reply {
        pub struct Sender<T>(pub core::marker::PhantomData<T>);
        impl Sender<system_api::control::frame::Said> {
            pub fn send(self, said: system_api::control::frame::Said) -> Result<(), ()> {
                crate::ANSWERS.with(|answers| answers.borrow_mut().push(said));
                Ok(())
            }
        }
    }
    pub mod request {
        pub struct Receiver<C>(core::marker::PhantomData<C>);
        pub struct Request {
            pub from: crate::TaskId,
            pub request: (Option<system_api::control::frame::Wire>, crate::PieToken),
            pub reply: super::reply::Sender<system_api::control::frame::Said>,
        }
        impl Receiver<system_api::control::Call> {
            pub fn from_raw(
                _: crate::PieToken,
                _: abi::Mark,
                _: impl Fn(
                    &(Option<system_api::control::frame::Wire>, crate::PieToken),
                ) -> crate::PieToken,
            ) -> Self {
                Self(core::marker::PhantomData)
            }
            pub fn receive(
                &self,
                _: &mut [u8],
                _: crate::Wait,
            ) -> Result<Request, super::Rejected> {
                Err(super::Rejected {
                    fail: super::Fail::Receive(()),
                })
            }
        }
    }
}
pub mod system {
    pub mod assembly {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct JobId(core::num::NonZeroU64);
        impl JobId {
            pub fn new(raw: u64) -> Option<Self> {
                core::num::NonZeroU64::new(raw).map(Self)
            }
            pub fn get(self) -> u64 {
                self.0.get()
            }
        }
        pub(crate) mod work {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../src/system/assembly/work.rs"
            ));
        }
        pub(crate) use work::WorkQueue;
        pub type Operations = WorkQueue<super::control::lifecycle::Tracked>;
        pub struct LocalCompletion {
            pub id: JobId,
            pub result: Option<Result<(), super::control::unit::verdict::Fail>>,
        }
        impl work::JobIdentity for LocalCompletion {
            fn completed_id(&self) -> Option<JobId> {
                self.result.as_ref().map(|_| self.id)
            }
        }
        pub struct Jobs {
            pub operations: Operations,
            pub completions: WorkQueue<LocalCompletion>,
        }
        impl Default for Jobs {
            fn default() -> Self {
                Self {
                    operations: WorkQueue::default(),
                    completions: WorkQueue::default(),
                }
            }
        }
        impl Jobs {
            pub fn take(&mut self, id: JobId) -> Option<LocalCompletion> {
                self.completions.take(id)
            }
            pub fn take_local(
                &mut self,
                id: JobId,
            ) -> Option<Result<(), super::control::unit::verdict::Fail>> {
                self.take(id).and_then(|completion| completion.result)
            }
            pub fn push_lifecycle(
                &mut self,
                mut request: super::control::lifecycle::Request,
            ) -> Result<
                JobId,
                (
                    super::control::unit::verdict::Fail,
                    super::control::lifecycle::Request,
                ),
            > {
                for work in self.operations.iter_mut() {
                    if work.operation.key.request.name != request.name {
                        continue;
                    }
                    if matches!(
                        (work.operation.key.request.action, request.action),
                        (
                            super::control::lifecycle::Action::Ruin,
                            super::control::lifecycle::Action::Ruin
                        )
                    ) {
                        if let Some(back) = request.back.take() {
                            work.operation.key.waiters.push(back);
                        }
                        return Ok(work.operation.key.id);
                    }
                    return Err((super::control::unit::verdict::Fail::NotReady, request));
                }
                let Some(id) = self.operations.reserve_job(272) else {
                    return Err((super::control::unit::verdict::Fail::Full, request));
                };
                self.operations
                    .push_back(super::control::lifecycle::Tracked::new(id, request));
                Ok(id)
            }
        }
    }
    pub mod app {
        #[derive(Debug)]
        pub enum Fault {
            Room,
            Shutdown,
        }
    }
    pub mod control {
        pub mod unit {
            pub mod start {
                pub const BOOT_MS: u32 = 3000;
            }
            pub mod table {
                pub use system_api::control::State;
                pub enum Slot {
                    None,
                    Live {
                        task: crate::TaskId,
                        team: Option<abi::TeamId>,
                    },
                }
                pub struct Service {
                    pub slot: Slot,
                    pub state: State,
                    pub name: String,
                }
                pub struct Table {
                    pub rows: Vec<Service>,
                }
                impl Table {
                    pub fn living(&self) -> impl Iterator<Item = &Service> {
                        self.rows.iter().filter(|row| row.state != State::Dead)
                    }
                }
            }
            pub mod verdict {
                #[derive(Clone, Copy, Debug)]
                pub enum Fail {
                    Unknown,
                    BadImage,
                    Full,
                    NotReady,
                }
                pub fn walking(_: &super::table::Table) -> bool {
                    false
                }
                pub fn due(_: &super::table::Table) -> bool {
                    false
                }
                pub fn done(_: &super::table::Table) -> bool {
                    false
                }
            }
            pub struct Control {
                pub table: table::Table,
                pub inputs: Vec<Input>,
                pub targets: &'static [crate::unit::Target],
            }
            pub struct Input {
                pub program: &'static crate::unit::UnitFile,
            }
            impl Control {
                pub fn input(&self, name: &str) -> Result<&Input, verdict::Fail> {
                    self.inputs
                        .iter()
                        .find(|input| input.program.name == name)
                        .ok_or(verdict::Fail::Unknown)
                }
                pub fn state(&self, _: String) -> Result<table::State, verdict::Fail> {
                    Ok(table::State::Ready)
                }
                pub fn task(&self, name: &str) -> Option<crate::TaskId> {
                    match self.table.rows.iter().find(|row| row.name == name)?.slot {
                        table::Slot::Live { task, .. } => Some(task),
                        table::Slot::None => None,
                    }
                }
                pub fn live(&self, task: crate::TaskId) -> bool {
                    !crate::unit::join_task(task, crate::Wait::POLL).unwrap_or(true)
                }
            }
            mod observe {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/observe.rs"
                ));
            }
        }
        pub mod lifecycle {
            #[derive(Clone, Copy)]
            pub enum Action {
                Mint,
                Embark { parent: Option<crate::TaskId> },
                Debark,
                Ruin,
            }
            pub struct Request {
                pub name: String,
                pub action: Action,
                pub back: Option<crate::rpc::reply::Sender<system_api::control::frame::Said>>,
            }
            #[cfg(test)]
            pub fn queued(
                operations: &crate::system::assembly::Operations,
            ) -> Vec<(String, Action, bool)> {
                operations
                    .iter()
                    .map(|job| {
                        (
                            job.operation.key.request.name.clone(),
                            job.operation.key.request.action,
                            job.operation.key.request.back.is_some(),
                        )
                    })
                    .collect()
            }
            pub struct Instance;
            pub struct Execution {
                pub instance: Option<Instance>,
                pub task: Option<crate::TaskId>,
                pub deadline: u64,
            }
            pub struct Operation {
                pub key: OperationKey,
                pub progress: ExecutionProgress,
            }
            pub struct OperationKey {
                pub id: crate::system::assembly::JobId,
                pub request: Request,
                pub waiters: Vec<crate::rpc::reply::Sender<system_api::control::frame::Said>>,
            }
            pub struct ExecutionProgress {
                pub execution: Execution,
                pub failure: Option<super::unit::verdict::Fail>,
            }
            pub struct Tracked {
                pub operation: Operation,
                pub cursor: schedule::Cursor,
                pub complete: bool,
            }
            impl Tracked {
                pub fn new(id: crate::system::assembly::JobId, request: Request) -> Self {
                    Self {
                        operation: Operation {
                            key: OperationKey {
                                id,
                                request,
                                waiters: Vec::new(),
                            },
                            progress: ExecutionProgress {
                                failure: None,
                                execution: Execution {
                                    instance: None,
                                    task: None,
                                    deadline: 0,
                                },
                            },
                        },
                        cursor: schedule::Cursor::default(),
                        complete: false,
                    }
                }
            }
        }
        pub mod namespace { pub use super::endpoint::Entries as Endpoints; }
        pub mod endpoint {
            pub struct Entries;
            impl Entries {
                pub fn face(&self, _: system_api::control::Grant) -> Option<crate::PieToken> {
                    None
                }
            }
            pub mod request {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/endpoint/request.rs"
                ));
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use system::control::{
        endpoint::request,
        unit::{
            Control,
            table::{Service, Slot, State, Table},
        },
    };
    use system_api::control::{Fail, frame};
    fn run(name: &str, task: usize, exited: bool) -> frame::Said {
        JOINED.with(|value| *value.borrow_mut() = exited);
        ANSWERS.with(|answers| answers.borrow_mut().clear());
        let mut resources = schedule::Resources::new();
        resources
            .insert(Control {
                inputs: Vec::new(),
                targets: &[],
                table: Table {
                    rows: vec![Service {
                        name: String::from("consumer"),
                        slot: Slot::Live {
                            task: TaskId::new(task),
                            team: None,
                        },
                        state: State::Ready,
                    }],
                },
            })
            .unwrap();
        resources
            .insert(request::Inbox(alloc::collections::VecDeque::from([
                request::Incoming {
                    wire: frame::Wire::Task(String::from(name)),
                    from: TaskId::new(9),
                    reply: rpc::reply::Sender(core::marker::PhantomData),
                },
            ])))
            .unwrap();
        request::state(resources.read().unwrap(), resources.write().unwrap()).unwrap();
        assert!(resources.read::<request::Inbox>().unwrap().0.is_empty());
        ANSWERS.with(|answers| answers.borrow_mut().pop().unwrap())
    }
    #[test]
    fn mutation_requests_enter_the_assembly_queue_with_the_native_parent() {
        use system::control::lifecycle::{Action, queued};
        ANSWERS.with(|answers| answers.borrow_mut().clear());
        let mut resources = schedule::Resources::new();
        resources.insert(system::assembly::Jobs::default()).unwrap();
        let incoming = |wire| request::Incoming {
            wire,
            from: TaskId::new(9),
            reply: rpc::reply::Sender(core::marker::PhantomData),
        };
        resources
            .insert(request::Inbox(alloc::collections::VecDeque::from([
                incoming(frame::Wire::Mint("provider".into())),
                incoming(frame::Wire::Embark("consumer".into())),
                incoming(frame::Wire::Mint("provider".into())),
            ])))
            .unwrap();
        request::enqueue(resources.write().unwrap(), resources.write().unwrap()).unwrap();
        assert!(resources.read::<request::Inbox>().unwrap().0.is_empty());
        let jobs = resources.read::<system::assembly::Jobs>().unwrap();
        let queued = queued(&jobs.operations);
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[0].0, "provider");
        assert!(matches!(queued[0].1, Action::Mint));
        assert_eq!(queued[1].0, "consumer");
        assert!(
            matches!(queued[1].1, Action::Embark { parent: Some(parent) } if parent == TaskId::new(9))
        );
        assert!(queued.iter().all(|job| job.2));
        ANSWERS.with(|answers| {
            assert_eq!(
                *answers.borrow(),
                [frame::said_status(frame::fail_to_code(Some(
                    Fail::NotReady
                )))]
            )
        });
    }

    #[test]
    fn shared_work_queue_does_not_burn_ids_when_capacity_rejects_admission() {
        let mut queue = system::assembly::WorkQueue::<u64>::default();
        let first = queue.reserve_job(1).unwrap();
        assert_eq!(first.get(), 1);
        queue.push_back(first.get());
        assert!(queue.reserve_job(1).is_none());
        queue.pop_front();
        let second = queue.reserve_job(1).unwrap();
        assert_eq!(second.get(), 2);
    }

    #[test]
    fn shared_take_requires_exact_completed_id_and_preserves_result_once() {
        use system::assembly::work::{Completed, JobIdentity};
        use system::assembly::JobId;
        enum Entry {
            Pending(JobId),
            Completed(Completed<Result<(), Fail>>),
        }
        impl JobIdentity for Entry {
            fn completed_id(&self) -> Option<JobId> {
                match self {
                    Self::Pending(_) => None,
                    Self::Completed(done) => Some(done.id),
                }
            }
        }
        let mut queue = system::assembly::WorkQueue::<Entry>::default();
        let first = queue.reserve_job(4).unwrap();
        let second = queue.reserve_job(4).unwrap();

        // A reserved ID has no result to take yet.
        queue.push_back(Entry::Pending(first));
        assert!(queue.take(first).is_none());
        queue.push_back(Entry::Completed(Completed { id: second, result: Err(Fail::Denied) }));
        assert!(queue.take(first).is_none());
        assert_eq!(queue.len(), 2);
        let Entry::Completed(completed) = queue.take(second).unwrap() else {
            panic!("the selected exact ID must be completed");
        };
        assert_eq!(completed.result, Err(Fail::Denied));
        assert!(queue.take(second).is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn shared_queue_rotates_unmatched_work_without_starving_other_jobs() {
        let mut queue = system::assembly::WorkQueue::<u8>::default();
        queue.extend([1, 2, 3]);
        assert_eq!(queue.pop_matching(3, |item| *item == 2), Some(2));
        assert_eq!(queue.iter().copied().collect::<Vec<_>>(), [3, 1]);
        assert_eq!(queue.pop_matching(2, |item| *item == 1), Some(1));
        assert_eq!(queue.pop_front(), Some(3));
    }

    #[test]
    fn named_query_returns_registered_live_task_from_control() {
        assert_eq!(
            run("consumer", 21, false),
            frame::said_task(TaskId::new(21))
        );
    }
    #[test]
    fn missing_exited_and_zero_tasks_never_become_successful_facts() {
        assert_eq!(
            run("unknown", 21, false),
            frame::said_status(frame::fail_to_code(Some(Fail::Unknown)))
        );
        assert_eq!(
            run("consumer", 21, true),
            frame::said_status(frame::fail_to_code(Some(Fail::NotReady)))
        );
        assert_eq!(
            run("consumer", 0, false),
            frame::said_status(frame::fail_to_code(Some(Fail::NotReady)))
        );
    }

    fn closing_fixture() -> Control {
        use unit::{ControlConfig, UnitFile};
        static HUB: UnitFile = UnitFile {
            name: "hub",
            control: ControlConfig { after: Some(&[]) },
        };
        static ROUTER: UnitFile = UnitFile {
            name: "router",
            control: ControlConfig {
                after: Some(&[crate::unit::Dependency::Unit("hub")]),
            },
        };
        static UART: UnitFile = UnitFile {
            name: "uart",
            control: ControlConfig {
                after: Some(&[
                    crate::unit::Dependency::Unit("hub"),
                    crate::unit::Dependency::Unit("router"),
                ]),
            },
        };
        static PIPE: UnitFile = UnitFile {
            name: "pipe",
            control: ControlConfig { after: Some(&[]) },
        };
        // Deliberately unrelated to startup or registration order.
        let programs = [&ROUTER, &PIPE, &UART, &HUB];
        Control {
            inputs: programs
                .iter()
                .map(|program| system::control::unit::Input { program })
                .collect(),
            targets: &[],
            table: Table {
                rows: programs
                    .iter()
                    .enumerate()
                    .map(|(i, program)| Service {
                        name: program.name.into(),
                        slot: Slot::Live {
                            task: TaskId::new(i + 10),
                            team: Some(abi::TeamId::new(i + 20)),
                        },
                        state: State::Ready,
                    })
                    .collect(),
            },
        }
    }
    fn state(control: &mut Control, name: &str, state: State) {
        control
            .table
            .rows
            .iter_mut()
            .find(|row| row.name == name)
            .unwrap()
            .state = state;
    }
    fn closing(control: &Control) -> Vec<&str> {
        control.closing_service_names().collect()
    }

    #[test]
    fn shutdown_orders_users_before_providers_and_allows_independent_services() {
        let mut control = closing_fixture();
        assert_eq!(closing(&control), ["pipe", "uart"]);
        state(&mut control, "uart", State::Dead);
        assert_eq!(closing(&control), ["router", "pipe"]);
        state(&mut control, "router", State::Dead);
        assert_eq!(closing(&control), ["pipe", "hub"]);
    }

    #[test]
    fn stopping_and_debarked_users_still_hold_their_providers() {
        let mut control = closing_fixture();
        state(&mut control, "uart", State::Stopping);
        assert_eq!(closing(&control), ["pipe"]);
        state(&mut control, "uart", State::Debarked);
        assert_eq!(closing(&control), ["pipe", "uart"]);
    }

    #[test]
    fn scene_users_stop_before_other_services_and_do_not_hold_each_other() {
        use unit::{ControlConfig, UnitFile};
        static FIRST: UnitFile = UnitFile {
            name: "first",
            control: ControlConfig {
                after: Some(&[crate::unit::Dependency::Target("scene")]),
            },
        };
        static LAST: UnitFile = UnitFile {
            name: "last",
            control: ControlConfig {
                after: Some(&[crate::unit::Dependency::Target("scene")]),
            },
        };
        static TARGETS: &[crate::unit::Target] = &[crate::unit::Target {
            name: "scene",
            members: &["hub"],
        }];
        let mut control = closing_fixture();
        control.targets = TARGETS;
        for program in [&FIRST, &LAST] {
            control
                .inputs
                .push(system::control::unit::Input { program });
            control.table.rows.push(Service {
                name: program.name.into(),
                slot: Slot::Live {
                    task: TaskId::new(30),
                    team: Some(abi::TeamId::new(40)),
                },
                state: State::Ready,
            });
        }
        assert_eq!(closing(&control), ["pipe", "uart", "first", "last"]);
        state(&mut control, "first", State::Stopping);
        assert_eq!(closing(&control), ["pipe", "uart", "last"]);
        state(&mut control, "first", State::Dead);
        state(&mut control, "last", State::Dead);
        assert_eq!(closing(&control), ["pipe", "uart"]);
        assert_eq!(closing(&control), ["pipe", "uart"]);
    }
}

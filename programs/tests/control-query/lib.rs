#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
pub use abi::{PieToken, TaskId, Wait};
use std::cell::RefCell;
thread_local! {
    static ANSWERS: RefCell<Vec<system_api::control::frame::Said>> = const { RefCell::new(Vec::new()) };
    static JOINED: RefCell<bool> = const { RefCell::new(false) };
}
pub mod unit {
    pub struct Relation { pub after: Option<&'static [&'static str]> }
    pub struct UnitFile { pub name: &'static str, pub relation: Relation }
    pub fn is_target(name: &str) -> bool { name == "scene" }
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
    pub mod app {
        #[derive(Debug)]
        pub enum Fault {
            Room,
        }
    }
    pub mod control {
        pub mod unit {
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
            }
            pub struct Input { pub program: &'static crate::unit::UnitFile }
            impl Control {
                pub fn input(&self, name: &str) -> Result<&Input, verdict::Fail> {
                    self.inputs.iter().find(|input| input.program.name == name)
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
            pub struct Operations;
            impl Operations {
                pub fn push(
                    &mut self,
                    _: Request,
                ) -> Result<(), (super::unit::verdict::Fail, Request)> {
                    Ok(())
                }
            }
            pub struct Execution {
                pub task: Option<crate::TaskId>,
            }
            pub struct Operation {
                pub request: Request,
                pub failure: Option<super::unit::verdict::Fail>,
                pub execution: Execution,
            }
        }
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
        use unit::{Relation, UnitFile};
        static HUB: UnitFile = UnitFile { name: "hub", relation: Relation { after: Some(&[]) } };
        static ROUTER: UnitFile = UnitFile { name: "router", relation: Relation { after: Some(&["hub"]) } };
        static UART: UnitFile = UnitFile { name: "uart", relation: Relation { after: Some(&["hub", "router"]) } };
        static PIPE: UnitFile = UnitFile { name: "pipe", relation: Relation { after: Some(&[]) } };
        // Deliberately unrelated to startup or registration order.
        let programs = [&ROUTER, &PIPE, &UART, &HUB];
        Control {
            inputs: programs.iter().map(|program| system::control::unit::Input { program }).collect(),
            table: Table { rows: programs.iter().enumerate().map(|(i, program)| Service {
                name: program.name.into(),
                slot: Slot::Live { task: TaskId::new(i + 10), team: Some(abi::TeamId::new(i + 20)) },
                state: State::Ready,
            }).collect() },
        }
    }
    fn state(control: &mut Control, name: &str, state: State) {
        control.table.rows.iter_mut().find(|row| row.name == name).unwrap().state = state;
    }
    fn closing(control: &Control) -> Vec<&str> { control.closing_service_names().collect() }

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
        use unit::{Relation, UnitFile};
        static FIRST: UnitFile = UnitFile { name: "first", relation: Relation { after: Some(&["scene"]) } };
        static LAST: UnitFile = UnitFile { name: "last", relation: Relation { after: Some(&["scene"]) } };
        let mut control = closing_fixture();
        for program in [&FIRST, &LAST] {
            control.inputs.push(system::control::unit::Input { program });
            control.table.rows.push(Service {
                name: program.name.into(),
                slot: Slot::Live { task: TaskId::new(30), team: Some(abi::TeamId::new(40)) },
                state: State::Ready,
            });
        }
        assert_eq!(closing(&control), ["first", "last"]);
        state(&mut control, "first", State::Stopping);
        assert_eq!(closing(&control), ["last"]);
        state(&mut control, "first", State::Dead);
        state(&mut control, "last", State::Dead);
        assert_eq!(closing(&control), ["pipe", "uart"]);
    }
}

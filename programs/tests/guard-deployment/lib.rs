#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as resource;

pub use abi::{
    Call, Mark, Name, Permission, PieInfo, PieKind, PieToken, TaskId, TeamId, UnitFail, Wait,
};
use std::{cell::RefCell, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub source: UnitFail,
}

#[derive(Default)]
pub struct Effects {
    pub calls: Vec<&'static str>,
    pub debark_fail: Option<UnitFail>,
    pub guard_fail: Option<UnitFail>,
    pub unguard_fail: Option<UnitFail>,
    pub embark_team_fail: Option<UnitFail>,
    pub embark_task_fail: Option<UnitFail>,
    pub oust_fail: Option<UnitFail>,
    pub joined: bool,
    pub inspect: Option<Result<PieInfo, ()>>,
}

thread_local! {
    pub static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default());
}

pub fn reset() {
    EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
}

pub fn calls() -> Vec<&'static str> {
    EFFECTS.with(|effects| effects.borrow().calls.clone())
}

fn record(call: &'static str) {
    EFFECTS.with(|effects| effects.borrow_mut().calls.push(call));
}

pub mod chrono {
    pub fn clock() -> u64 {
        0
    }
}

pub mod unit {
    pub fn self_id() -> crate::TaskId {
        crate::TaskId::new(1)
    }

    pub fn debark_team(_: crate::TeamId) -> Result<(), crate::Error> {
        crate::record("debark_team");
        match crate::EFFECTS.with(|effects| effects.borrow().debark_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    pub fn guard(_: crate::TeamId, _: crate::PieToken) -> Result<(), crate::Error> {
        crate::record("guard");
        match crate::EFFECTS.with(|effects| effects.borrow().guard_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    pub fn unguard(_: crate::TeamId, _: crate::PieToken) -> Result<(), crate::Error> {
        crate::record("unguard");
        match crate::EFFECTS.with(|effects| effects.borrow().unguard_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    pub fn embark_team(_: crate::TeamId) -> Result<(), crate::Error> {
        crate::record("embark_team");
        match crate::EFFECTS.with(|effects| effects.borrow().embark_team_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    pub fn embark_task(_: crate::TaskId) -> Result<(), crate::Error> {
        crate::record("embark_task");
        match crate::EFFECTS.with(|effects| effects.borrow().embark_task_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    pub fn slay_team(_: crate::TeamId) -> Result<(), crate::Error> {
        crate::record("slay_team");
        crate::EFFECTS.with(|effects| effects.borrow_mut().joined = true);
        Ok(())
    }

    pub fn join_task(_: crate::TaskId, _: crate::Wait) -> Result<bool, crate::Error> {
        crate::record("join_task");
        Ok(crate::EFFECTS.with(|effects| effects.borrow().joined))
    }

    pub fn oust(_: crate::TeamId) -> Result<(), crate::Error> {
        crate::record("oust");
        match crate::EFFECTS.with(|effects| effects.borrow().oust_fail) {
            Some(source) => Err(crate::Error { source }),
            None => Ok(()),
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Ending {
        Resident,
    }

    #[derive(Clone, Copy)]
    pub struct Retirement;

    #[derive(Clone, Copy)]
    pub struct ControlConfig {
        pub retirement: &'static [Retirement],
    }

    #[derive(Clone, Copy)]
    pub struct UnitFile {
        pub control: ControlConfig,
    }
}

pub mod ledger {
    pub mod manifest {
        pub const MAX_PROGRAMS: usize = 64;
    }
}

pub mod room {
    pub fn doom(_: crate::TaskId) -> Result<(), ()> {
        crate::record("doom");
        Ok(())
    }
}

pub mod raw {
    pub fn inspect(token: crate::PieToken) -> Result<crate::PieInfo, ()> {
        crate::EFFECTS.with(|effects| {
            effects
                .borrow()
                .inspect
                .unwrap_or(Err(()))
                .map(|mut info| {
                    info.token = token;
                    info
                })
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuardSelectionError {
    Unknown,
    Duplicate,
    Unstarted,
}

pub mod guard_selection {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/unit/guard_selection.rs"
    ));
}

pub mod app {
    pub mod guard {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/system/app/guard.rs"
        ));
    }
}

pub mod system {
    pub mod control {
        pub mod unit {
            pub mod verdict {
                #[derive(Clone, Copy, PartialEq, Eq, Debug)]
                pub enum Fail {
                    Unknown,
                    Full,
                    NotReady,
                }
            }

            pub mod table {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/table.rs"
                ));
            }

            pub mod guard {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/guard.rs"
                ));
            }

            use alloc::{string::String, vec::Vec};

            #[derive(Clone, Copy)]
            pub struct Input {
                pub name: &'static str,
                pub program: &'static crate::unit::UnitFile,
            }

            pub struct Pending {
                pub name: String,
            }

            pub struct Control {
                pub table: table::Table,
                pub inputs: Vec<Input>,
                pub pending: Vec<Pending>,
                pub guard_token: Option<crate::PieToken>,
            }

            impl Control {
                pub fn input(&self, name: &str) -> Result<Input, verdict::Fail> {
                    self.inputs
                        .iter()
                        .copied()
                        .find(|input| input.name == name)
                        .ok_or(verdict::Fail::Unknown)
                }

                pub fn task(&self, name: &str) -> Option<crate::TaskId> {
                    match self.table.find(name)?.slot {
                        table::Slot::Live { task, .. } => Some(task),
                        table::Slot::None => None,
                    }
                }

                pub fn guard_held(&self, name: &str) -> bool {
                    self.table.guard_held(name)
                }

                pub fn clear_guard(&mut self, name: &str) {
                    self.table.set_guard_held(name, false);
                }

                pub fn detach_unit(&mut self, name: &str) -> Result<(), verdict::Fail> {
                    self.table.detach(name)?;
                    self.table.set_state(name, table::State::Dead);
                    Ok(())
                }
            }

            pub static PROGRAM: crate::unit::UnitFile = crate::unit::UnitFile {
                control: crate::unit::ControlConfig { retirement: &[] },
            };
        }

        pub mod identity {
            pub struct Roster;

            impl Roster {
                pub fn install_id(&self, _: crate::TaskId) -> Option<u64> {
                    None
                }

                pub fn retire_install(&mut self, _: u64) -> Result<(), ()> {
                    Ok(())
                }
            }
        }

        pub struct Construction;

        impl Construction {
            pub fn retire<F>(&mut self, _: crate::TaskId, f: F) -> Result<(), ()>
            where
                F: FnOnce(u64, u64) -> Result<(), ()>,
            {
                f(0, 0)
            }
        }

        pub mod instance {
            pub mod state {
                pub struct Instance {
                    pub identity_install: Option<u64>,
                }
            }
        }

        pub mod lifecycle {
            use alloc::string::String;

            pub struct Active(pub Option<Operation>);

            pub struct Operation {
                pub key: OperationKey,
                pub progress: ExecutionProgress,
            }

            pub struct OperationKey {
                pub request: Request,
            }

            pub struct Request {
                pub name: String,
                pub action: Action,
            }

            pub enum Action {
                Ruin,
            }

            pub struct ExecutionProgress {
                pub execution: Execution,
                pub failure: Option<()>,
            }

            pub struct Execution {
                pub instance: Option<super::instance::state::Instance>,
                pub task: Option<crate::TaskId>,
                pub deadline: u64,
            }

            pub mod ruin {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/ruin.rs"
                ));
            }
        }
    }

    pub mod app {
        pub mod policy {
            pub struct Settle {
                pub settled: bool,
            }

            impl Settle {
                pub fn gate(&mut self, _: &crate::unit::UnitFile) -> bool {
                    self.settled
                }
            }
        }

        pub mod supplies {
            pub struct Supplies;

            impl Supplies {
                pub fn release_target(&mut self, _: crate::TaskId) -> Result<(), ()> {
                    Ok(())
                }
            }
        }
    }

    pub mod assembly {
        pub struct Jobs;

        impl Jobs {
            pub fn retire_create_scope(&mut self, _: u64, _: u64) -> Result<(), ()> {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;

#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as resource;

pub use abi::{
    Call, Mark, Name, Permission, PieInfo, PieKind, PieToken, TaskId, TeamId, UnitFail, Wait,
};

#[path = "fixtures/kernel.rs"]
mod kernel;
#[path = "fixtures/effects.rs"]
mod effects;
#[path = "fixtures/unit_calls.rs"]
mod unit_calls;
#[path = "fixtures/unit/unit_file.rs"]
mod unit_file;
#[path = "fixtures/unit/control_config.rs"]
mod control_config;
#[path = "fixtures/unit/ready.rs"]
mod ready;
#[path = "fixtures/verdict.rs"]
mod verdict;
#[path = "fixtures/control/control.rs"]
mod control_type;
#[path = "fixtures/control/input.rs"]
mod input;
#[path = "fixtures/control/pending.rs"]
mod pending;
#[path = "fixtures/control/service.rs"]
mod service;
#[path = "fixtures/control/readiness.rs"]
mod readiness;
#[path = "fixtures/control/roster.rs"]
mod roster;
#[path = "fixtures/control/construction.rs"]
mod construction;
#[path = "fixtures/lifecycle/active.rs"]
mod active;
#[path = "fixtures/lifecycle/action.rs"]
mod action;
#[path = "fixtures/lifecycle/execution.rs"]
mod execution;
#[path = "fixtures/lifecycle/execution_progress.rs"]
mod execution_progress;
#[path = "fixtures/lifecycle/instance.rs"]
mod instance;
#[path = "fixtures/lifecycle/operation.rs"]
mod operation;
#[path = "fixtures/lifecycle/operation_key.rs"]
mod operation_key;
#[path = "fixtures/lifecycle/request.rs"]
mod request;
#[path = "fixtures/app/settle.rs"]
mod settle;
#[path = "fixtures/app/supplies.rs"]
mod supplies;
#[path = "fixtures/assembly/jobs.rs"]
mod jobs;
#[path = "fixtures/setup.rs"]
mod setup;

pub use effects::{calls, reset, Effects, EFFECTS};
pub(crate) use effects::record;
pub use kernel::Error;
pub use unit_calls::Ending;
pub use unit_file::UnitFile;
pub use control_config::ControlConfig;
pub use ready::Ready;
pub use verdict::Fail;
pub use control_type::Control;
pub use input::Input;
pub use pending::Pending;
pub use service::Service;
pub use readiness::Readiness;
pub use roster::Roster;
pub use construction::Construction;
pub use active::Active;
pub use action::Action;
pub use execution::Execution;
pub use execution_progress::ExecutionProgress;
pub use instance::Instance;
pub use operation::Operation;
pub use operation_key::OperationKey;
pub use request::Request;
pub use settle::Settle;
pub use supplies::Supplies;
pub use jobs::Jobs;
pub use setup::*;

pub use kernel::{chrono, debug, ledger, raw, room};

pub mod unit {
    pub use crate::unit_calls::*;
    pub use crate::unit_file::UnitFile;
    pub use crate::control_config::ControlConfig;
    pub use crate::ready::Ready;
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
            pub use crate::control_type::Control;
            pub use crate::input::Input;
            pub use crate::pending::Pending;
            pub use crate::service::Service;

            pub mod task {
                pub use crate::readiness::Readiness;
            }

            pub mod table {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/table.rs"
                ));
            }

            pub mod verdict {
                pub use crate::verdict::Fail;
            }

            pub mod guard {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/guard.rs"
                ));
            }
        }

        pub mod identity {
            pub use crate::roster::Roster;
        }

        pub use crate::construction::Construction;

        pub mod lifecycle {
            pub use crate::action::Action;
            pub use crate::active::Active;
            pub use crate::execution::Execution;
            pub use crate::execution_progress::ExecutionProgress;
            pub use crate::instance::Instance;
            pub use crate::operation::Operation;
            pub use crate::operation_key::OperationKey;
            pub use crate::request::Request;

            pub mod origin {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/origin.rs"
                ));
            }
            pub use origin::Origin;

            pub mod directive {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/directive.rs"
                ));
            }
            pub use directive::Directive;

            pub mod intent {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/intent.rs"
                ));
            }

            pub mod authority {
                use ::schedule::Progress;
                pub fn verify_effect(
                    _: &crate::Active,
                    _: &crate::Control,
                    _: &crate::Settle,
                ) -> Result<(), crate::Fail> {
                    Ok(())
                }
                pub fn recheck(
                    _: ::schedule::Res<crate::Active>,
                    _: ::schedule::Res<crate::Control>,
                    _: ::schedule::Res<crate::Settle>,
                ) -> Result<Progress, crate::Fail> {
                    Ok(Progress::Done)
                }
            }

            pub mod embark {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/embark.rs"
                ));
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
            pub use crate::settle::Settle;
        }

        pub mod supplies {
            pub use crate::supplies::Supplies;
        }
    }

    pub mod assembly {
        pub use crate::jobs::Jobs;
    }
}

#[cfg(test)]
mod tests;

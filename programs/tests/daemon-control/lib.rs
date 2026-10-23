#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as programs;

use std::cell::RefCell;

thread_local! {
    static DEAD: RefCell<Vec<TaskId>> = const { RefCell::new(Vec::new()) };
    static ANSWERS: RefCell<Vec<system_api::control::frame::Said>> = const { RefCell::new(Vec::new()) };
}

pub mod chrono {
    pub fn clock() -> u64 {
        100
    }
}
pub mod debug {
    pub fn put(_: &str) {}
}
pub use abi::{PieToken, TaskId, TeamId, Wait};

pub mod ledger {
    pub mod manifest {
        pub const MAX_PROGRAMS: usize = 256;
    }
}

pub mod support {
    #[cfg(test)]
    pub use crate::cases::support::*;

    pub mod timing {
        pub const BOOT_MS: u32 = 3000;
    }
}

pub mod fixtures;

pub mod unit {
    use crate::{DEAD, TaskId, Wait};

    pub use crate::fixtures::{ControlConfig, Dependency, Ending, Target, UnitFile};

    pub fn self_id() -> TaskId {
        TaskId::new(1)
    }

    pub fn join_task(task: TaskId, _: Wait) -> Result<bool, ()> {
        Ok(DEAD.with(|dead| dead.borrow().contains(&task)))
    }
}

pub mod rpc {
    pub mod reply {
        pub use crate::fixtures::Sender;
    }
}

pub mod system {
    pub mod app {
        pub use crate::fixtures::Fault;

        pub mod policy {
            pub use crate::fixtures::Settle;
        }
    }

    pub mod control {
        pub mod endpoint {
            pub mod request {
                pub fn reply(
                    back: crate::rpc::reply::Sender<system_api::control::frame::Said>,
                    said: system_api::control::frame::Said,
                ) {
                    let _ = back.send(said);
                }
            }
        }

        pub mod unit {
            pub use crate::fixtures::{Control, Input};

            pub mod start {
                pub use super::Input;
                pub const BOOT_MS: u32 = 3000;
            }
            pub mod table {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/table.rs"
                ));
            }
            pub mod verdict {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/verdict.rs"
                ));
            }
            pub mod intent {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/unit/intent.rs"
                ));
            }
        }

        pub mod supervision {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../src/system/control/supervision.rs"
            ));
        }

        pub mod lifecycle {
            pub use crate::fixtures::{
                Action, Active, Execution, ExecutionProgress, Instance, Operation, OperationKey,
                Request, Tracked,
            };
            pub use crate::system::control::unit::verdict::Fail;

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

            pub mod transition {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/transition.rs"
                ));
            }

            pub mod authority {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/authority.rs"
                ));
            }

            pub mod authority_verify {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/authority_verify.rs"
                ));
            }

            pub mod takeover {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/takeover.rs"
                ));
            }

            pub mod finish {
                include!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src/system/control/lifecycle/finish.rs"
                ));
            }
        }
    }
}

#[cfg(test)]
mod cases;

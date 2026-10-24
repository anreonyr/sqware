pub mod peers;

#[path = "../../../src/service/hub/serve/launch_phase.rs"]
pub mod launch_phase;
#[path = "../../../src/service/hub/serve/launch_reply.rs"]
pub mod launch_reply;
#[path = "../../../src/service/hub/serve/return_reply.rs"]
pub mod return_reply;
#[path = "../../../src/service/hub/serve/launch.rs"]
pub mod launch;
#[path = "../../../src/service/hub/serve/returned.rs"]
pub mod returned;
#[path = "../../../src/service/hub/serve/execution.rs"]
pub mod execution;

#[path = "../../../src/service/hub/serve/binding_rpc.rs"]
mod binding_rpc;
#[path = "../../../src/service/hub/serve/coordination.rs"]
mod coordination;
#[cfg(test)]
mod adapter_cases;

use core::time::Duration;
use env::Wait;
use ipc::time::{deadline, remain};
use system_api::control::Fail;
use system_api::control::observe::Reply;
use system_client::control::Observe;

const RPC_MS: usize = 250;
const POLL_MS: usize = 2;

/// Observe one page position within one fixed RPC budget.
pub fn observe(face: &Observe, index: u32) -> Result<Reply, Fail> {
    let wait = Wait::AtMost(RPC_MS);
    let until = deadline(wait);
    let mut pending = face.begin_observe(index, wait)?;
    loop {
        if let Some(reply) = face.poll_observe(&mut pending)? {
            return Ok(reply);
        }
        if remain(until) == Wait::POLL {
            return Err(Fail::Bad);
        }
        let _ = execution::room::park(Duration::from_millis(POLL_MS as u64));
    }
}

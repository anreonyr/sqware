use alloc::string::String;
use core::time::Duration;
use env::Wait;
use ipc::time::{deadline, remain};
use system_api::control::Fail;
use system_client::control::Face;

const RECOVER_MS: usize = 5000;
const POLL_MS: usize = 2;

/// Ask for one conditional recovery and wait for the actual restart result.
pub fn recover(face: &Face, name: String) -> Result<(), Fail> {
    let wait = Wait::AtMost(RECOVER_MS);
    let until = deadline(wait);
    let mut pending = face.begin_recover(name, wait)?;
    loop {
        if face.poll_recover(&mut pending)?.is_some() {
            return Ok(());
        }
        if remain(until) == Wait::POLL {
            return Err(Fail::Bad);
        }
        let _ = execution::room::park(Duration::from_millis(POLL_MS as u64));
    }
}

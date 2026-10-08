//! Capability delivery follows the request's admission and reply outcome.

use env::{Mark, PieToken, TaskId};
use ipc::session::CallFail;
use resource::{
    port::{Access, Policy},
    raw::Loan,
};
use system_api::operator::Said;

/// The accepted result transfers responsibility to the server. A lost reply leaves
/// admission uncertain, so it cannot revoke a capability the server may have retained.
pub(super) fn offer(
    source: &PieToken,
    host: TaskId,
    call: impl FnOnce(PieToken) -> Result<Said, CallFail>,
) -> Result<Said, ()> {
    let loan = Loan::accord(
        source,
        host,
        Access::FETCH.bits() | Access::STORE.bits() | Policy::VEST.bits(),
        Mark::NONE,
    )
    .map_err(|_| ())?;
    match call(loan.remote()) {
        Ok(response) => {
            if response.failure_status().is_none() {
                loan.keep();
            }
            Ok(response)
        }
        Err(CallFail::Receive(_)) => {
            loan.keep();
            Err(())
        }
        Err(_) => Err(()),
    }
}

//! Capability delivery follows the request's admission and reply outcome.

use env::{Mark, PieToken, TaskId};
use ipc::session::CallFail;
use resource::{
    port::{Access, Policy},
    raw::Loan,
};

/// The accepted result transfers responsibility to the server. A lost reply leaves
/// admission uncertain, so it cannot revoke a capability the server may have retained.
pub(super) fn offer<T>(
    source: &PieToken,
    host: TaskId,
    call: impl FnOnce(PieToken) -> Result<(bool, T), CallFail>,
) -> Result<T, ()> {
    let loan = Loan::accord(
        source,
        host,
        Access::FETCH.bits() | Access::STORE.bits() | Policy::VEST.bits(),
        Mark::NONE,
    )
    .map_err(|_| ())?;
    match call(loan.remote()) {
        Ok((accepted, response)) => {
            if accepted {
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

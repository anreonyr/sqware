use crate::core::work::room::scheduler;

use super::current;

pub(super) fn dispatch() -> Option<usize> {
    // Ready tasks must not escape every scheduler container between commits.
    let _commit = crate::core::work::commit();
    let s = current();
    s.pull()
        .or_else(scheduler::steal)
        .and_then(|task| s.seat(task))
}

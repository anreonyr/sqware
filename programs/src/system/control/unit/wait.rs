use super::verdict::Reaped;
use env::{TaskId, Wait};

pub(super) fn until(task: TaskId, wait: Wait) -> Reaped {
    if env::unit::join(task, Wait::POLL).unwrap_or(true) {
        return Reaped::Now;
    }
    if wait == Wait::POLL {
        return Reaped::Unsettled;
    }
    let _ = env::unit::join(task, wait);
    if env::unit::join(task, Wait::POLL).unwrap_or(true) {
        Reaped::Waited
    } else {
        Reaped::Unsettled
    }
}

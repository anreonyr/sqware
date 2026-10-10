use crate::runtime::diagnose::halt::hush;
use crate::work::room::scheduler::core::{current, fetch};

pub fn run() -> usize {
    hush();
    let commit = crate::work::unit::commit();
    if current().running_task().is_some_and(|task| crate::work::room::messenger::take_doomed(task.ident.id).is_some()) {
        drop(commit);
        return crate::work::room::messenger::quit();
    }
    let next = current().advance();
    drop(commit);
    match next {
        Some(pa) => pa,
        None => fetch(),
    }
}

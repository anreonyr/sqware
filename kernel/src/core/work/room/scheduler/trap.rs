use crate::core::work::room::scheduler::core::{current, fetch};
use crate::machine::stop::hush;

pub fn run() -> usize {
    hush();
    let commit = crate::core::work::unit::commit();
    if current()
        .running_task()
        .is_some_and(|task| crate::core::work::room::messenger::take_doomed(task.ident.id).is_some())
    {
        drop(commit);
        return crate::core::work::room::messenger::quit();
    }
    let next = current().advance();
    drop(commit);
    match next {
        Some(pa) => pa,
        None => fetch(),
    }
}

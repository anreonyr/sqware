use env::{PieFail, PieToken};

use super::cull;
use crate::work::unit::task::Task;

pub(crate) fn release(
    task: &alloc::sync::Arc<Task>,
    token: PieToken,
) -> Result<usize, PieFail> {
    let graph = super::GRAPH.lock();
    let source = super::locate(task, token).ok_or(PieFail::Denied)?;
    let _operation = if let super::AnyPie::Pole(p) = &source {
        let operation = p.meta().backing().operation().ok_or(PieFail::Busy)?;
        if p.meta().backing().reserved() != 0 { return Err(PieFail::Busy); }
        Some(operation)
    } else { None };
    let snap = super::snap();
    let present = {
        let pies = task.pies.lock();
        pies.iter().any(|p| p.token() == token)
    };
    if !present {
        return Err(PieFail::Denied);
    }
    let cleanup = cull::cull((task.clone(), token), &snap);
    drop(graph);
    Ok(cleanup.finish())
}

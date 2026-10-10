use env::{TaskId, Wait};
use system_api::identity::{CoalitionId, CoalitionSet, Reply, Wire};
/// Activate eligible coalitions through the exact startup Identity authority.
pub fn activate(
    face: &system_client::identity::Face,
    task: TaskId,
    coalitions: &[CoalitionId],
) -> Result<(), ()> {
    let coalitions = CoalitionSet::new(coalitions).map_err(|_| ())?;
    match face
        .call(Wire::Activate(task, coalitions), Wait::AtMost(5000))
        .map_err(|_| ())?
    {
        Reply::Unit => Ok(()),
        _ => Err(()),
    }
}

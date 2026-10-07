//! Supervisor publication of Operator's request faces.
use ::schedule::{Progress, Res, ResMut};
use crate::system::{boot::Mounts, common::face::mount, life::Status, run::publication::Internal};
use alloc::sync::Arc;
use system_client::operator as op;
pub fn faces(
    status: Res<Arc<Status>>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    for grant in op::Grant::ALL {
        let (entry, _) = mount::entry(grant.mark(), grant.name())?;
        let permit = if matches!(grant, op::Grant::Part | op::Grant::Land | op::Grant::Trim) {
            op::Permit::Bound
        } else {
            op::Permit::Public
        };
        mounts.0.push(Internal {
            road: op::DIR.try_join(grant.name()).ok_or("Operator path")?,
            entry,
            access: (permit, status.control),
        });
    }
    Ok(Progress::Done)
}

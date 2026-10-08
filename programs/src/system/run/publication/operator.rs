//! Supervisor publication of Operator's request faces.
use ::schedule::{Progress, Res, ResMut};
use crate::system::{boot::Mounts, common::face::mount, life::Status, run::publication::Internal};
use alloc::sync::Arc;
pub fn faces(
    status: Res<Arc<Status>>,
    mut mounts: ResMut<Mounts>,
) -> Result<Progress, &'static str> {
    for grant in system_api::operator::Grant::ALL {
        let (entry, _) = mount::entry(grant.mark(), grant.name())?;
        let permit = if matches!(grant, system_api::operator::Grant::Part | system_api::operator::Grant::Land | system_api::operator::Grant::Trim) {
            system_api::operator::Permit::Bound
        } else {
            system_api::operator::Permit::Public
        };
        mounts.0.push(Internal {
            road: system_api::operator::DIR.try_join(grant.name()).ok_or("Operator path")?,
            entry,
            access: (permit, status.control),
        });
    }
    Ok(Progress::Done)
}

use crate::system::{
    boot::Mounts, common::face::mount, control::serve::publication::Internal, life::Status,
};
use alloc::sync::Arc;
use protocol::{
    common::schedule::{Progress, Res, ResMut},
    system::operator as op,
};
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

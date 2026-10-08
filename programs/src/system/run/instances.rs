use crate::system::control::{Fail, unit::Control};
use ::schedule::{Progress, Res, ResMut};
use env::unit;
use system_api::control as call;

pub fn reap(mut control: ResMut<Control>, flow: Res<super::frame::Flow>) -> Result<Progress, Fail> {
    control.reap_instances(flow.settling);
    Ok(Progress::Done)
}
pub fn pending(
    control: Res<Control>,
    mut bound: ResMut<super::frame::Bound>,
) -> Result<Progress, Fail> {
    bound.0 = control.instance_wait(bound.0);
    Ok(Progress::Done)
}

pub fn publication(
    mut watch: ResMut<crate::system::control::serve::watch::Watch>,
    mut mounts: ResMut<crate::system::boot::Mounts>,
) -> Result<Progress, &'static str> {
    let entry = env::pie::unseal_hole(call::ASK_MARK).map_err(|_| "instance entry")?;
    watch.instance = Some(entry);
    mounts.0.push(crate::system::run::publication::Internal {
        road: call::INSTANCE.to_path_buf(),
        entry,
        access: (system_api::operator::Permit::Bound, unit::self_id()),
    });
    Ok(Progress::Done)
}

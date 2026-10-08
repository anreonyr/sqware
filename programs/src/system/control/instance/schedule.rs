use crate::system::app::Fault as Fail;
use crate::system::control::unit::Control;

use ::schedule::{Progress, Res, ResMut};

pub fn reap(
    mut control: ResMut<Control>,
    flow: Res<crate::system::app::policy::Flow>,
) -> Result<Progress, Fail> {
    control.reap_instances(flow.settling);
    Ok(Progress::Done)
}
pub fn pending(
    control: Res<Control>,
    mut bound: ResMut<crate::system::app::policy::Bound>,
) -> Result<Progress, Fail> {
    bound.0 = control.instance_wait(bound.0);
    Ok(Progress::Done)
}

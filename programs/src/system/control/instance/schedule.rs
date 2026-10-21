use crate::system::app::Fault as Fail;
use crate::system::app::policy::Settle;
use crate::system::control::unit::Control;

use ::schedule::{Progress, Res, ResMut};

pub fn reap(mut control: ResMut<Control>, settle: Res<Settle>) -> Result<Progress, Fail> {
    // 声明的退休依赖全部完成后，才允许全局收回执行实例。
    control.reap_instances(settle.flow.settling);
    Ok(Progress::Done)
}
pub fn pending(
    control: Res<Control>,
    mut bound: ResMut<crate::system::app::policy::Bound>,
) -> Result<Progress, Fail> {
    bound.0 = control.instance_wait(bound.0);
    Ok(Progress::Done)
}

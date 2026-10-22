use crate::system::app::Fault as Fail;
use crate::system::app::policy::Settle;
use crate::system::control::unit::Control;

use ::schedule::{Progress, Res, ResMut};

pub fn reap(mut control: ResMut<Control>, settle: Res<Settle>) -> Result<Progress, Fail> {
    // 关闭新建准入后即停止动态执行域，退休提供方最后才结清。
    control.reap_instances(settle.flow.settling || settle.closing());
    Ok(Progress::Done)
}
pub fn pending(
    control: Res<Control>,
    mut bound: ResMut<crate::system::app::policy::Bound>,
) -> Result<Progress, Fail> {
    bound.0 = control.instance_wait(bound.0);
    Ok(Progress::Done)
}

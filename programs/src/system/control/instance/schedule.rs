use crate::system::app::Fault as Fail;
use crate::system::app::policy::Settle;
use crate::system::control::unit::Control;

use ::schedule::{Progress, Res, ResMut};

pub fn reap(mut control: ResMut<Control>, settle: Res<Settle>) -> Result<Progress, Fail> {
    // 退场门：只有 Hub 结清全部绑定之后 `settling` 才为真，故这里不会抢在
    // 恢复责任结清之前把执行实例收掉。
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

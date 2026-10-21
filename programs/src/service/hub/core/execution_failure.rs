use env::{HoleFail, PieFail, UnitFail};
use system_api::control::Fail;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionFailure {
    Control(Fail),
    Hole(HoleFail),
    Pie(PieFail),
    Unit(UnitFail),
}

use env::{HoleFail, PieFail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortOpenFail {
    /// 创建本地回信孔失败。
    Hole(HoleFail),
    /// 查询入口或授出回信孔失败。
    Pie(PieFail),
}

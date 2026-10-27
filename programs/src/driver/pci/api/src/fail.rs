//! 本手自己的失败词汇：每一格对应一个不同的下一步。

use wire::OK;

#[derive(Clone, Copy, PartialEq, Eq, Debug, env::WireCodes)]
#[wire(fallback = Bad)]
pub enum Fail {
    /// 这一号绑定不在本域记的落位里。
    #[code(1)]
    Unknown,
    /// 同一号绑定的参数与已落位那一笔不一致。
    #[code(2)]
    Conflict,
    /// 本域拒了（设备不在、窗口放不下、状态不允许）。
    #[code(3)]
    Denied,
    /// 这一手不成：回读与写入对不上。
    #[code(4)]
    Unavailable,
    /// 本端读不懂那一句。
    #[code(5)]
    Bad,
}

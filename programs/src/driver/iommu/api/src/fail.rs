//! 本手自己的失败词汇：每一格对应一个不同的下一步。

use wire::OK;

/// 本手自己的失败词汇：每一格对应一个不同的下一步。
#[derive(Clone, Copy, PartialEq, Eq, Debug, env::WireCodes)]
#[wire(fallback = Bad)]
pub enum Fail {
    /// 这一号绑定不在本 Domain 记的账上。
    #[code(1)]
    Unknown,
    /// 同一号绑定的参数与已登记那一笔不一致（或 Domain 正被别的绑定占着）。
    #[code(2)]
    Conflict,
    /// 本域拒了（Domain 不在、来源窗不在、权限不够、区间非法）。
    #[code(3)]
    Denied,
    /// 硬件没按协议回答（回读不对、fence 超期、队列不停）。
    #[code(4)]
    Unavailable,
    /// 本端读不懂那一句。
    #[code(5)]
    Bad,
    /// 执行方还没把窗交到本域表里：这一步还没就绪，不是失败。
    #[code(6)]
    NotReady,
    /// 私有 IOVA 空间耗尽。
    #[code(7)]
    Full,
}

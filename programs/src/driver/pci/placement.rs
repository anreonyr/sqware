//! 一枚已落位的端点：绑定号 ＋ 落位事实 ＋ 交给恢复方的那一枚窗口。
//!
//! 它**不是第二份分配账**：`address`/`size` 就是 `Endpoint::place` 那一手的事实，
//! `bar` 是恢复方表里的那一枚号（本域念不动它，只负责把它交给协调入口）。

use env::{PieToken, TaskId};

use crate::driver::pci::virtio::Caps;

pub struct Placement {
    pub serial: u64,
    pub address: u64,
    pub size: u64,
    pub pages: u32,
    pub caps: Caps,
    pub recovery: TaskId,
    /// 恢复方表里那一枚精确 BAR token。
    pub bar: PieToken,
    /// MASTER 此刻是开的吗；落位之后、`enable` 之前恒为关。
    pub master: bool,
    /// 开过一次又被关了吗——关过就不再重开（重复 `enable` 不能使 Closing 绑定重新开放）。
    pub disabled: bool,
}

impl Placement {
    pub fn pages(&self) -> u32 {
        self.pages
    }
}

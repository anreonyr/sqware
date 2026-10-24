//! 一枚已落位的端点：绑定号 ＋ 落位事实 ＋ 交给恢复方的那一枚窗口。
//!
//! 它**不是第二份分配账**：`address`/`size` 就是 `Endpoint::place` 那一手的事实，
//! `bar` 是恢复方表里的那一枚号（本域念不动它，只负责把它交给协调入口）。

use env::PieToken;

pub struct Placement {
    pub address: u64,
    pub size: u64,
    pub pages: u32,
    /// 恢复方表里那一枚精确 BAR token。
    pub bar: PieToken,
}

impl Placement {
    pub fn pages(&self) -> u32 {
        self.pages
    }
}

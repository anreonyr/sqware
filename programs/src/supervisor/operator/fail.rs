//! operator 这一域的**错误类型**——`serve()` 的返回类型，`?` 一路把它带出来。
//!
//! 号与从前的 `const E_*` **同值**，只是现在有类型、能带话；调用点那两句 `say(...)`
//! 由 [`Exit::report`] 那一句话接手（内核在出口当场打，不需要控制台活着）。

use crate::{Exit, Report};

/// 本域的死法：**一格 = 死在起手的哪一步**（`Room` 是唯一不按"步"分的：起手要的那一页备不下）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Sire,
    Tip,
    Group,
    /// 起手要备的那**一页**（收帧的缓冲，见 `server.rs` 的 `serve`）备不下 ⇒ 这道门起不来。
    ///
    /// **照实记（为什么不并进 `Group`）**：它落在同一步的尾巴上，但它是**资源**那一格，不是那一
    /// 步本身；四个兄弟域各有自己的 `Desk` 收这同一种失败，而本域的起手名单里没有"门"这一步，
    /// 故为它单开一格。
    Room,
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            Fail::Sire => 1,
            Fail::Tip => 2,
            Fail::Group => 3,
            Fail::Room => 4,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Sire => "operator: no sire",
            Fail::Tip => "operator: tip",
            Fail::Group => "operator: no group",
            Fail::Room => "operator: no room",
        }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code(), self.text())
    }
}

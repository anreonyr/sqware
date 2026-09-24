//! operator 这一域的**错误类型**——`serve()` 的返回类型，`?` 一路把它带出来。
//!
//! 号与从前的 `const E_*` **同值**，只是现在有类型、能带话；调用点那两句 `say(...)`
//! 由 [`Exit::report`] 那一句话接手（内核在出口当场打，不需要控制台活着）。

use crate::{Exit, Report};

/// 本域的死法：**一格 = 死在起手的哪一步**（号与从前的 `const E_*` 同值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Sire,
    Tip,
    Group,
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            Fail::Sire => 1,
            Fail::Tip => 2,
            Fail::Group => 3,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Sire => "operator: no sire",
            Fail::Tip => "operator: tip",
            Fail::Group => "operator: no group",
        }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code(), self.text())
    }
}

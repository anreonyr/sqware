//! principal 这一域的**错误类型**——`serve()` 的返回类型，`?` 一路把它带出来。
//!
//! 号与从前的 `const E_*` **同值**，只是现在有类型、能带话；调用点那两句 `say(...)`
//! 由 [`Exit::report`] 那一句话接手（内核在出口当场打，不需要控制台活着）。

use crate::{Exit, Report};

/// 本域的死法：**一格 = 死在起手的哪一步**（号与从前的 `const E_*` 同值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Sire,
    Board,
    Tree,
    Book,
    Desk,
}

impl Fail {
    pub fn code(self) -> env::Reason {
        match self {
            Fail::Sire => 1,
            Fail::Board => 2,
            Fail::Tree => 3,
            Fail::Book => 4,
            Fail::Desk => 5,
        }
    }

    pub const fn text(self) -> &'static str {
        match self {
            Fail::Sire => "principal: no sire",
            Fail::Board => "principal: board",
            Fail::Tree => "principal: tree",
            Fail::Book => "principal: no book",
            Fail::Desk => "principal: desk",
        }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code(), self.text())
    }
}

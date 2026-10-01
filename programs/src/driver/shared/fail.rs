//! 三台共用一枚扁平的死法（号取自装配表）。

use crate::unit::Died;
use crate::{Exit, Report};

/// 一台驱动的死法：**一个号 ＋ 那一句话**（`main` 的返回类型），`?` 一路把它带出来
pub struct Fail {
    code: Died,
    text: &'static str,
}

impl Fail {
    /// 死在**某一步**：号是装配表里那一号（[E_UART](programs::unit::uart::E_UART) 那种
    pub const fn at(code: Died, text: &'static str) -> Self {
        Self { code, text }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code, self.text)
    }
}

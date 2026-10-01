//! driver::fail — **一台驱动怎么死**：三台共用一枚扁平的死法（号取自装配表）。
//! ```text
//!   Fail   { code, text }：一个号 + 那一句话 —— `main` 的返回类型
//!   号     各域在装配表上那一号，**名字就是那个名字**（`programs::unit::uart::E_UART`）；
//!          "配给那一趟没成"带 `assemble` 那一族的号（原样带）
//! ```

use crate::unit::Died;
use crate::{Exit, Report};

/// 一台驱动的死法：**一个号 ＋ 那一句话**（`main` 的返回类型），`?` 一路把它带出来。
pub struct Fail {
    code: Died,
    text: &'static str,
}

impl Fail {
    /// 死在**某一步**：号是装配表里那一号（[E_UART](programs::unit::uart::E_UART) 那种，
    /// 或本域自己那几条步名），那句话是**步名**（`"tree"` / `"desk"` 那种，
    /// 见上面那一格裁）。
    pub const fn at(code: Died, text: &'static str) -> Self {
        Self { code, text }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code, self.text)
    }
}

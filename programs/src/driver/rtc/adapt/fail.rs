//! rtc::adapt::fail — 本域的**死法**：`main` 的返回类型，`?` 一路把它带出来。
//!
//! 号与从前的 `const E_*` **同值**（1–3 归 [`assemble`]，4 起是本域），只是现在有类型、能带话。
//!
//! **它与 [`crate::core::fail`] 是两件事**（路径把它们分开了）：这一枚是**下线**的那一格
//! ——"这一域死在起手/常驻的哪一步"，读的人是内核出口与板那条死亡道；那一枚是**上线**的
//! 那一格——"客人那一问怎么了"，折成答码过线。故它住适配层（`Exit` 是程序侧的事）。

use programs::{Exit, Report};

/// 时钟驱动的死法：**一格 = 死在启动/常驻的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 环境调用失败（`sire` / 开会话 / 铸孔这一类）。
    /// `assemble::receive` 带来的号（原样往外带）。
    Assemble(env::Reason),
    /// 那一页寄存器开不动。
    Open,
    /// 上板那三步。
    Board,
    /// 上树那一趟（门牌 / 会话 / 问话孔）。
    Tree,
    /// 占线（坐标与登记那一趟）。
    Line,
    /// 那只组（门上的请求与线上的投递）。
    Desk,
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            // 环境负码的**样子**照实带出去：`usize` 是 64 位，负码在自己那段高位上仍互不相同。
            Fail::Assemble(code) => code,
            Fail::Open => 4,
            Fail::Board => 5,
            Fail::Line => 6,
            Fail::Tree => 7,
            Fail::Desk => 8,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Assemble(_) => "rtc: assemble",
            Fail::Open => "rtc: device open failed",
            Fail::Board => "rtc: board",
            Fail::Tree => "rtc: tree",
            Fail::Line => "rtc: line",
            Fail::Desk => "rtc: desk",
        }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code(), self.text())
    }
}



impl From<env::Reason> for Fail {
    fn from(code: env::Reason) -> Self {
        Fail::Assemble(code)
    }
}

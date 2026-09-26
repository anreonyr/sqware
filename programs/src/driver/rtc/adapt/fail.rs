//! rtc::adapt::fail — 本域的死法：**一格 = 死在启动/常驻的哪一步**（一族口径在 [`driver::fail`]）。
//!
//! 本文件只留 rtc 自己的事实：**它走得到哪几步**、每一步那句话，以及它在装配单上那一号
//! （`plan::assembly::E_RTC`——**本域一个数都不写**，见 `driver/fail.rs` 那条照实记）。
//!
//! **它与 `rtc::core::Fail` 是两件事**：这一枚是**下线**那一格（"这一域死在起手/常驻的哪一步"，
//! 读的人是内核出口与板那条死亡道）；那一枚是**上线**那一格（"客人那一问怎么了"，折成答码过线）。
//!
//! [`driver::fail`]: programs::driver::fail

use plan::assembly::{Died, E_RTC};
use programs::driver::fail::{self, Who};

/// rtc 这一台（[`Fail`] 里那格"谁"）。
pub struct Rtc;

/// 时钟驱动的死法：**一格 = 死在启动/常驻的哪一步**（只有 rtc 走得到的那几格）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 环境调用失败（`sire` / 开会话 / 铸孔这一类）。
    /// `assemble::receive` 带来的号（原样往外带）。
    Assemble(Died),
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

impl fail::Step for Step {
    fn code(self, died: Died) -> Died {
        match self {
            // 配给那一趟的号**原样带过**；本域自己那几格取装配单里那一号。
            Step::Assemble(code) => code,
            _ => died,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Step::Assemble(_) => "rtc: assemble",
            Step::Open => "rtc: device open failed",
            Step::Board => "rtc: board",
            Step::Tree => "rtc: tree",
            Step::Line => "rtc: line",
            Step::Desk => "rtc: desk",
        }
    }
}

impl Who for Rtc {
    type Step = Step;
    const DIED: Died = E_RTC;

    fn assembled(code: Died) -> Step {
        Step::Assemble(code)
    }
}

/// 本域的死法（`main` 的返回类型）。
pub type Fail = fail::Fail<Rtc>;

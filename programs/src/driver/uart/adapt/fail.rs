//! uart::adapt::fail — 本域的死法：**一格 = 死在启动/常驻的哪一步**（一族口径在 [`driver::fail`]）。
//!
//! 本文件只留 uart 自己的事实：**它走得到哪几步**、每一步那句话，以及它在装配单上那一号
//! （`plan::assembly::E_UART`——**本域一个数都不写**，见 `driver/fail.rs` 那条照实记）。
//!
//! [`driver::fail`]: programs::driver::fail

use plan::assembly::{Died, E_UART};
use programs::driver::fail::{self, Who};

/// uart 这一台（[`Fail`] 里那格"谁"）。
pub struct Uart;

/// uart 的死法：**一格 = 死在启动/常驻的哪一步**（只有 uart 走得到的那几格）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 环境调用失败（`sire` / 开会话 / 铸孔这一类）。
    /// `assemble::receive` 带来的号（`E_UP` / `E_GRANT`——原样往外带）。
    Assemble(Died),
    /// 设备门开不动 / 坐标不是区（`Dock::open`、`key.base()`）。
    Open,
    /// 上板那三步（开板路、要问话孔、交接）。
    Board,
    /// 上树那一趟（开树路、要孔、解门牌）。
    Tree,
    /// 登记那条线（`register` 那一趟）。
    Line,
    /// 常驻循环里那个域（`receive` 失败 ⇒ 这个域没有可继续的状态）。
    Dead,
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
            Step::Assemble(_) => "uart: assemble",
            Step::Open => "uart: device open failed",
            Step::Board => "uart: board",
            Step::Tree => "uart: tree",
            Step::Line => "uart: line",
            Step::Dead => "uart: line gone",
        }
    }
}

impl Who for Uart {
    type Step = Step;
    const DIED: Died = E_UART;

    fn assembled(code: Died) -> Step {
        Step::Assemble(code)
    }
}

/// 本域的死法（`main` 的返回类型）。
pub type Fail = fail::Fail<Uart>;

//! router::adapt::fail — 本域的死法：**一格 = 死在启动/常驻的哪一步**（一族口径在 [`driver::fail`]）。
//!
//! 本文件只留 router 自己的事实：**它走得到哪几步**、每一步那句话，以及它在装配单上那一号
//! （`plan::assembly::E_ROUTER`——**本域一个数都不写**，见 `driver/fail.rs` 那条照实记）。
//!
//! [`driver::fail`]: programs::driver::fail

use plan::assembly::{Died, E_ROUTER};
use programs::driver::fail::{self, Who};

/// router 这一台（[`Fail`] 里那格"谁"）。
pub struct Router;

/// 线路由者的死法：**一格 = 死在启动/常驻的哪一步**（只有 router 走得到的那几格）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 环境调用失败（`sire` / 铸孔 / 开会话这一类）。
    /// `assemble::receive` 带来的号（`E_UP` / `E_GRANT`——原样往外带）。
    Assemble(Died),
    /// 两枚门闩开不动（控制器 / 树）。
    Open,
    /// 读树那一趟没读出控制器、context 与线集合。
    Tree,
    /// 账备不下这台控制器的格子（**拒起**，不是运行期降级）。
    Account,
    /// 服务入口（那枚门牌孔）。
    Desk,
    /// 那只组（铃 / 门上有人 / 排空三合一的等待）。
    Bell,
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
            Step::Assemble(_) => "router: assemble",
            Step::Open => "router: docks",
            Step::Tree => "router: tree",
            Step::Account => "router: line account full",
            Step::Desk => "router: desk",
            Step::Bell => "router: bell",
        }
    }
}

impl Who for Router {
    type Step = Step;
    const DIED: Died = E_ROUTER;

    fn assembled(code: Died) -> Step {
        Step::Assemble(code)
    }
}

/// 本域的死法（`main` 的返回类型）。
pub type Fail = fail::Fail<Router>;

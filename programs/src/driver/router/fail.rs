//! router 这一域的**错误类型**——`main` 的返回类型，`?` 一路把它带出来。
//!
//! 号与从前的 `const E_*` **同值**（1–3 归 [`assemble`]，4 起是本域），只是现在有类型、能带话。

use programs::{Exit, Report};

/// 线路由者的死法：**一格 = 死在启动/常驻的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 环境调用失败（`sire` / 铸孔 / 开会话这一类）。
    /// `assemble::receive` 带来的号（`E_UP` / `E_GRANT`——原样往外带）。
    Assemble(env::Reason),
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

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            // 环境负码的**样子**照实带出去：`usize` 是 64 位，负码在自己那段高位上仍互不相同。
            Fail::Assemble(code) => code,
            Fail::Open => 4,
            Fail::Tree => 5,
            Fail::Bell => 6,
            Fail::Desk => 7,
            Fail::Account => 8,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Assemble(_) => "router: assemble",
            Fail::Open => "router: docks",
            Fail::Tree => "router: tree",
            Fail::Bell => "router: bell",
            Fail::Desk => "router: desk",
            Fail::Account => "router: line account full",
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

//! uart 这一域的**错误类型**——`main` 的返回类型，`?` 一路把它带出来。
//!
//! 它就是从前那几张 `const E_*` 编号表换了个样子：**号没变**（`Died` 那一族还是小整数，
//! 读 trace 的人照样一眼看出死在第几步），只是现在
//!   - **有类型**：忘了一个格子是编译错误（从前 `exit_with(6)` 谁也拦不住）；
//!   - **能带话**：退场那一句 note 与号长在一起（`Report::note`），内核在出口当场打；
//!   - **能过 `?`**：`EnvError` 与 `assemble` 那一族的号各有 `From`，调用点不必再拆
//!     `Err(code) => return code`。
//!
//! 编号口径：`2..=3` 是 [`assemble`] 那一族（`1` 是已撤的 `E_SIRE`），`4..` 起是本域自己的。

use env::EnvError;
use programs::{Exit, Report};

/// uart 的死法：**一格 = 死在启动/常驻的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 环境调用失败（`sire` / 开会话 / 铸孔这一类）。
    Env(EnvError),
    /// `assemble::receive` 带来的号（`E_UP` / `E_GRANT`——原样往外带）。
    Assemble(env::Reason),
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

/// 本域的小整数编号（**与从前的 `const E_*` 同值**，只是搬进了类型里）。
impl Fail {
    fn code(self) -> env::Reason {
        match self {
            // 环境负码的**样子**照实带出去：`usize` 是 64 位，负码在自己那段高位上仍互不相同。
            Fail::Env(e) => e.code() as env::Reason,
            Fail::Assemble(code) => code,
            Fail::Open => 4,
            Fail::Board => 5,
            Fail::Line => 6,
            Fail::Tree => 7,
            Fail::Dead => 8,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Env(_) => "uart: envcall",
            Fail::Assemble(_) => "uart: assemble",
            Fail::Open => "uart: device open failed",
            Fail::Board => "uart: board",
            Fail::Tree => "uart: tree",
            Fail::Line => "uart: line",
            Fail::Dead => "uart: line gone",
        }
    }
}

impl Exit for Fail {
    fn report(&self) -> Report<'_> {
        Report::note(self.code(), self.text())
    }
}

impl From<EnvError> for Fail {
    fn from(e: EnvError) -> Self {
        Fail::Env(e)
    }
}

/// `?` 那条路上有两种包装：裸的 [`EnvError`]（`env` 层的转发）与 `erra::Error<EnvError>`
/// （`runtime`/`protocol` 那两层包过的）。两者都收进同一格——号的账是同一本（`EnvError::code`）。
impl From<erra::Error<EnvError>> for Fail {
    fn from(e: erra::Error<EnvError>) -> Self {
        Fail::Env(e.into_source())
    }
}

impl From<env::Reason> for Fail {
    fn from(code: env::Reason) -> Self {
        Fail::Assemble(code)
    }
}

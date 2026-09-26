//! driver::fail — **一台驱动怎么死**：三台共用这一格口径（**号取自装配单**）。
//!
//! ```text
//!   Step   死在起手/常驻的**哪一步**——各域自己那一枚枚举实现它（只列自己走得到的那几格）
//!   Who     **哪一台**——决定号（[`Who::DIED`]）与"配给那一趟没成"那一格长什么样
//!   Fail    一台驱动的死法：`main` 的返回类型，`?` 一路把它带出来
//! ```
//!
//! 从前三台各有一份 `adapt/fail.rs`（三份逐字同构：同款的 derive、同款的 `Assemble(env::Reason)`、
//! 同款的 `impl Exit` 与 `impl From`，三份共 198 行）。这一刀并成族一级一枚，与
//! [`tree`](crate::driver::tree) / [`register`](crate::driver::register) /
//! [`assemble`](crate::driver::assemble) 同一条判据：**同构才收**。
//!
//! # 照实记（并的理由不是"少写三遍"，是"两套号在跑"）
//!
//! 装配单里 `E_ROUTER` / `E_UART` / `E_RTC` 说的是"**这一台**死了"，而三台自己那几格从前写的是
//! `4..=8`——**同一件事两套号**，与内件那三枚当年一模一样（那一刀记在
//! `crates/plan/src/assembly.rs` 的 `E_TREE` / `E_PRINCIPAL` / `E_COALITION` 那一段照实记里：
//! 三份同构的 `fail::Fail` 并成 `system::Start` 一枚，号也收进那张表）。
//!
//! 故本表**一个数都不写**：本域那几步一律报 [`Who::DIED`]，"死在第几步"留在那一句话里
//! （`"uart: line gone"`）。唯一自己带号的是"**配给那一趟没成**"那一格——它带的是装配那一族的号
//! （`assemble::E_UP` / `assemble::E_GRANT`），**原样往外带**（折成同一个号就等于把那几个编号
//! 变成没人读得到的死码）。
//!
//! # 照实记（读数因此变了）
//!
//! uart 起手各步 `4..=8` → **`9`**（`E_UART`）、router → **`5`**（`E_ROUTER`）、
//! rtc → **`12`**（`E_RTC`）；**那句话一个字没变**，故 trace 里仍一眼看出死在哪一步。
//!
//! # 两枚 `fail` 是两件事
//!
//! 本表是**下线**那一格——"这一域死在起手/常驻的哪一步"，读的人是内核出口与板那条死亡道；
//! `rtc::core::Fail` 是**上线**那一格——"客人那一问怎么了"，折成答码过线（`Taken` / `Past` /
//! `Denied`）。故本表住适配侧（[`Exit`] 是程序侧那一手），而它**不是**服务面的失败域。

use crate::{Exit, Report};
use plan::assembly::Died;

/// 死在起手/常驻的**哪一步**：各域自己那一枚枚举实现它（**只列自己走得到的那几格**）。
pub trait Step: Copy {
    /// 这一格报什么号：**本域那几步一律取装配单里那一号**（`died` 那一格），唯一自己带号的是
    /// "配给那一趟没成"。
    fn code(self, died: Died) -> Died;
    /// 交给内核出口的那句话（带本域名，如 `"uart: line gone"`）。
    fn text(self) -> &'static str;
}

/// **哪一台驱动**：决定号（取自装配单）与"配给那一趟没成"那一格。
pub trait Who {
    /// 本域自己那几格——只有它走得到的那些。
    type Step: Step;
    /// **号取自装配单**：本域一个数都不写（`plan::assembly` 那一族）。
    const DIED: Died;
    /// "配给那一趟没成"那一格：`?` 把装配那一族的号交给它。
    fn assembled(code: Died) -> Self::Step;
}

/// 一台驱动的死法：`main` 的返回类型，`?` 一路把它带出来。
///
/// 三台是**同一个类型**（只是 `W` 不同）：`Fail<Uart>` / `Fail<Router>` / `Fail<Rtc>`——
/// 各自的 `Fail` 是各域那一份 `pub type`，格集仍只有自己那几格。
pub struct Fail<W: Who> {
    step: W::Step,
}

impl<W: Who> Fail<W> {
    /// 死在**本域那几步**之一。
    pub fn at(step: W::Step) -> Self {
        Self { step }
    }
}

impl<W: Who> Exit for Fail<W> {
    fn report(&self) -> Report<'_> {
        Report::note(self.step.code(W::DIED), self.step.text())
    }
}

/// `assemble::receive` 那一族的号（"装配的哪一步没成"）由这里过 `?`。
impl<W: Who> From<Died> for Fail<W> {
    fn from(code: Died) -> Self {
        Self::at(W::assembled(code))
    }
}

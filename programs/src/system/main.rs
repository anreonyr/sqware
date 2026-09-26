#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把一样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）；此外一概不给。
//!
//! ```text
//! 1  起手：与引导域搭会话 + 领机器自述 + 领载荷区清单（`bootstrap::take`）
//! 2  这一景起哪些台：`scenario::rows`（就是装配单 `plan::assembly::ALL` 里这一景真有的那些）
//! 3  登记整张账（`System::enlist`：名字 + 怎么算起来）
//! 4  逐条起（`System::bring_up`）：建域 → 装通道 → 身份 → 放行等就绪 → 领配给 → 上板 → 接树
//! 5  监督（`System::supervise`）：谁没了 ⇒ 记账 + 放下那个死域
//! 6  最后一条没了 ⇒ 对仍在跑的显式 `stop` ⇒ 全部记完 ⇒ 收场
//! 7  本域退出 ⇒ 引导域那枚孔随之封印 ⇒ 它退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**，而且只有编排者这一条：三枚服务（持树者 / 名册 / 盟册）各自是一个
//! 程序、一个域（`src/system/{operator,principal,coalition}/main.rs`），由本域按装配单
//! 用与其他每一台相同的 `mint` 起起来——**没有 `Role` 那种"同一份字节按 args 分派"的特例**。

extern crate alloc;
extern crate programs;

use env::Name;
use programs::system::assemble as scenario;
use programs::system::bootstrap;
use programs::system::control::{self, E_MANIFEST, E_PROGRAM, E_TABLE};
use programs::system::program::Program;
use programs::system::System;

/// 本域的死法：**一格 = 死在起手的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 与引导域那条会话没搭上。
    Firmware,
    /// 那台机器的自述（`Key::dtb`）没领到 / 读不懂。
    Machine,
    /// 那块载荷区（清单在里面）没领到 / 读不懂。
    Payload,
    /// 清单那一条读不懂。
    Manifest,
    /// 死亡道那只组。
    Group,
    /// 整表装配那一趟带来的号（**按服务分的号取自装配单**，由 `System` 折出）。
    Assemble(env::Reason),
    /// 监督那一趟。
    Supervise,
    /// 收尾那一趟。
    Doom,
}

/// 起手那几格的号 / 说法都由 `bootstrap` 那一族持有（与从前的值逐格相同）。
impl From<bootstrap::Fail> for Fail {
    fn from(f: bootstrap::Fail) -> Fail {
        match f {
            bootstrap::Fail::Firmware => Fail::Firmware,
            bootstrap::Fail::Machine => Fail::Machine,
            bootstrap::Fail::Payload => Fail::Payload,
            bootstrap::Fail::Manifest => Fail::Manifest,
        }
    }
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            Fail::Firmware => bootstrap::Fail::Firmware.code(),
            Fail::Machine => bootstrap::Fail::Machine.code(),
            Fail::Payload => bootstrap::Fail::Payload.code(),
            Fail::Manifest => bootstrap::Fail::Manifest.code(),
            Fail::Assemble(code) => code,
            Fail::Group => control::E_TABLE,
            Fail::Supervise => 8,
            Fail::Doom => 9,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::Firmware => bootstrap::Fail::Firmware.text(),
            Fail::Machine => bootstrap::Fail::Machine.text(),
            Fail::Payload => bootstrap::Fail::Payload.text(),
            Fail::Manifest => bootstrap::Fail::Manifest.text(),
            Fail::Assemble(_) => "system: assemble",
            Fail::Group => "system: no group",
            Fail::Supervise => "system: supervise",
            Fail::Doom => "system: doom",
        }
    }
}

impl programs::Exit for Fail {
    fn report(&self) -> programs::Report<'_> {
        programs::Report::note(self.code(), self.text())
    }
}

#[programs::entry]
fn main() -> programs::Report<'static> {
    match system() {
        Ok(()) => programs::Report::new(env::EXIT_OK),
        Err(f) => programs::Report::note(f.code(), f.text()),
    }
}

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办。
fn system() -> Result<(), Fail> {
    // 1. 起手三样：与引导域那条会话、机器自述、载荷区清单（配给与镜像都从它们来）。
    let boot = bootstrap::take().map_err(Fail::from)?;

    // 2. 这一景起哪些台（**按 `order` 排**：先起的先就绪，后面的就能向它要东西）。
    let rows = scenario::rows(&boot.catalog);
    let Some(last_row) = rows.last() else {
        return Err(Fail::Assemble(E_PROGRAM));
    };
    let last = Name::new(last_row.name).map_err(|_| Fail::Assemble(E_MANIFEST))?;

    // 死亡道跟着装配单铸：上板的那几位一位一条——在 `System::new` 里。
    let mut sys = System::new(boot, &rows).map_err(|_| Fail::Group)?;

    // 3. 登记：先立账（名字 + 怎么算起来），身子要等真的起了才挂上。
    for row in &rows {
        let program = Program {
            name: row.name,
            setup: scenario::setup_of(row.name),
        };
        sys.enlist(&program).map_err(|_| Fail::Assemble(E_TABLE))?;
    }

    // 4. 逐条起。`Service` 那本通道账**不必抱着**：孔归本域那张表（`Endpoint` 上没有"放下"这个
    //    动作，谁拿都不改变归属），
    //    起完就不指着它了——装配者往后只通过板 / 树那两条路与它说话。
    for row in &rows {
        let program = Program {
            name: row.name,
            setup: scenario::setup_of(row.name),
        };
        let plan = row.plan.as_ref().expect("scenario::rows 已滤过 plan");
        sys.bring_up(&program, plan).map_err(Fail::Assemble)?;
    }

    // 5/6. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    sys.supervise(last);
    // 7. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    Ok(())
}

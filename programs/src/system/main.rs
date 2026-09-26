#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把一样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）；此外一概不给。
//!
//! ```text
//! 1  起手：与引导域搭会话 + 领机器自述 + 领载荷区清单（`bootstrap::take`）
//! 2  节点与边：`scenario::nodes` 给出这一景有哪些 Program、它们在图里是什么关系
//! 3  登记整张账（`System::enlist`：名字 + 怎么算起来）
//! 4  逐条起（`System::bring_up`）：spawn → 身份 → 放行等就绪 → 领配给 → 上板 → 接树
//! 5  监督（`control::supervise::run`）：谁没了 ⇒ 记账 + 放下那个死域
//! 6  最后一条没了 ⇒ 对仍在跑的显式 `stop` ⇒ 全部记完 ⇒ 收场
//! 7  本域退出 ⇒ 引导域那枚孔随之封印 ⇒ 它退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**：起谁、按什么顺序、开哪几条通道、要哪些门闩、上不上板、上不上树，
//! 全在 `system/assemble/`（节点）与 `system/mod.rs`（边怎么落）。要加第三个服务 ——
//! 单里加一行，本文件一个字不改。

extern crate alloc;
extern crate programs;

use env::Name;
use programs::system::bootstrap;
use programs::system::control::service as server;
use programs::system::control::supervise;
use programs::system::control::{self, E_MANIFEST, E_PROGRAM, E_TABLE};
use programs::system::program::Role;
use programs::system::{coalition, operator, principal, System};

// 节点 / 边 / 内件表住 lib 的 `system/assemble/`（scenario）。
use programs::system::assemble as scenario;

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
    /// 整表装配那一趟带来的号（**按服务分的号取自装配单**，由 System 折出）。
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

/// **四种角色共用的出口**：`Ok` 报 [`EXIT_OK`](env::EXIT_OK)，`Err` 那一格原样往上递。
fn exit(role: Result<(), programs::Report<'static>>) -> programs::Report<'static> {
    match role {
        Ok(()) => programs::Report::new(env::EXIT_OK),
        Err(r) => r,
    }
}

/// **本域的死法 → 出口那一格**（两句话都是常量：`code()` + `text()`）。
///
/// 三枚内件走 [`server::said`](server::said)——它们共用 `control::service::Start` 那一枚死法类型。
fn said(f: Fail) -> programs::Report<'static> {
    programs::Report::note(f.code(), f.text())
}

#[programs::entry]
fn main() -> programs::Report<'static> {
    // **一枚 ELF 四种角色**：角色由 `Spawn` 那一格 `args` 递进来（[`Role`]）。空 args
    // ⇒ **编排域自己那一枚**。
    match Role::of_args(runtime::core::unit::args()) {
        Role::System => exit(system().map_err(said)),
        Role::Tree => exit(operator::server::serve().map_err(server::said)),
        Role::Roster => exit(principal::server::serve().map_err(server::said)),
        Role::League => exit(coalition::server::serve().map_err(server::said)),
    }
}

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办。
///
/// 返 `Result<(), Fail>`：本域的死法有类型（[`Fail`]），折成出口那一格是 [`main`] 那一步的事。
fn system() -> Result<(), Fail> {
    // 1. 起手三样：与引导域那条会话、机器自述、载荷区清单（配给与镜像都从它们来）。
    let boot = bootstrap::take().map_err(Fail::from)?;

    // 2. 节点与边：内件三枚在前、镜像里那几台在后（`scenario::nodes` 给次序）。
    let nodes = scenario::nodes(&boot.catalog);
    let Some(last_node) = nodes.last() else {
        return Err(Fail::Assemble(E_PROGRAM));
    };
    let last = Name::new(last_node.program.name).map_err(|_| Fail::Assemble(E_MANIFEST))?;

    // 死亡道跟着边铸：上板的那几位一位一条（本域铸、记号 `gone-<名字>`）——在 `System::new` 里。
    let mut sys = System::new(boot, &nodes).map_err(|_| Fail::Group)?;

    // 3. 登记：先立账（名字 + 怎么算起来），身子要等真的起了才挂上。
    for node in &nodes {
        sys.enlist(node).map_err(|_| Fail::Assemble(E_TABLE))?;
    }

    // 4. 逐条起。**顺序即契约**：先起的先就绪，后面的就能向它要东西。
    for node in &nodes {
        sys.bring_up(node).map_err(Fail::Assemble)?;
    }

    // 5/6. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    supervise::run(sys.control_mut(), last);
    // 7. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    Ok(())
}

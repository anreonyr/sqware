#![no_std]
#![no_main]

//! system — **编排域**：这台机器上有哪些服务、怎么起、谁死了怎么办。
//!
//! 它是 boot 之后**唯一**起服务的地方。引导域（`root`）只把一样东西交给它：**这块字节**
//! （清单 + 全部镜像，一枚只读门闩）；此外一概不给。
//!
//! ```text
//! 1  起手：与引导域搭会话 + 领机器自述 + 领载荷区清单（`bootstrap::take`）
//! 2  这一景起哪些台：`assemble::programs`（**过滤 + 按各台声明的边算次序**，就这一件事）
//! 3  逐条起：`Assembly::assemble`——每一台按**它自己那份声明**装配（相与手见 `system::schedule`）
//! 4  **这一趟走完**（`program::SCENE` 那一格到点）：挂 `control` 那一面
//!    （`Assembly::mount_control`）→ 监督那一趟（`Assembly::supervise`）：谁没了 ⇒ 记账 + 放下
//!    那个死域；**该收了**就逐位下刀，**收讫了**才收场（判据在 `Control::{due, done}` 上）
//! 5  本域退出 ⇒ 引导域那枚孔随之封印 ⇒ 它退出 ⇒ 级联扑杀 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**，而且只有编排者这一条：四枚服务（持树者 / 名册 / 盟册 / 设备账）各自是一个
//! 程序、一个域（`src/system/{operator,principal,coalition}/main.rs`），由本域按那张装配表
//! 用与其他每一台相同的 `mint` 起起来——**没有 `Role` 那种"同一份字节按 args 分派"的特例**。
//!
//! **这里不再有 `UnitFile { … }` 那样的投影**：声明是各台自己那份 `program.rs`，本文件只把
//! `&list` 交给 [`Assembly::assemble`](crate::system::Assembly::assemble)。

extern crate alloc;
extern crate programs;

use programs::system::Assembly;
use programs::system::assemble;
use programs::system::bootstrap;
use programs::system::control::{self, E_PROGRAM};

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
    /// 整表装配那一趟带来的号（**按服务分的号取自那一台自己的 `died`**，由 `Assembly::assemble` 折出）。
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

    // 2. 这一景起哪些台（**次序由各台声明里的 `after` 算出来**：先起的先就绪，后面的就能向它要东西）。
    let list = assemble::programs(&boot.catalog).map_err(|_| Fail::Assemble(E_PROGRAM))?;
    // **空单**：这一景一台可装配的都没有 ⇒ 报那一格（"有单可装"是下面每一趟的前提）。
    if list.is_empty() {
        return Err(Fail::Assemble(E_PROGRAM));
    }

    // 死亡道跟着这张单铸：要存在信号的那几位一位一条——在 `Assembly::new` 里。
    let mut assembly = Assembly::new(boot).map_err(|_| Fail::Group)?;

    // 3. 逐条起：**每一台按它自己那份声明装配**（立账 → 建域产线程 → 装通道 → 身份 → 放行等
    //    就绪 → 递配给 → 存在信号 → 树），失败带的是**那一台自己的号**。
    //    `Service` 那本通道账不必抱着：孔归本域那张表（`Endpoint` 上没有"放下"这个动作，谁拿
    //    都不改变归属），起完就不指着它了——装配者往后只通过板 / 树那两条路与它说话。
    for program in &list {
        assembly.assemble(program).map_err(Fail::Assemble)?;
    }

    // 4. **这一趟走完**（`programs::program::SCENE` 那一格到点）：**先把 `control` 那一面挂上树**
    //    ——那一刻起那一面才有人待客（`probe-control` 等的就是这一件事，它靠 `after` 里那条
    //    `SCENE` 边排到最后一位）；然后进监督那一趟。
    assembly.mount_control();
    //    监督那一趟：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；**该收了**就下刀，**收讫了**才收场
    //    （判据都在 `Control` 上：`due` / `done`，见 `control/supervise.rs` 的头注）。
    //    返 `false` = 有人没收讫 ⇒ 报"收尾那一趟没走完"，余下交退场级联。
    if !assembly.supervise() {
        return Err(Fail::Doom);
    }
    // 5. 本域退出 ⇒ 引导域那枚孔封印 ⇒ 它退出 ⇒ 级联 ⇒ 停机。
    Ok(())
}

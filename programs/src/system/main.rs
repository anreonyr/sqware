#![no_std]
#![no_main]


extern crate alloc;
extern crate programs;

use programs::system::Assembly;
use programs::system::run::scene;
use programs::system::run::bootstrap;
use programs::system::control::{self, E_PROGRAM};

/// 本域的死法：**一格 = 死在起手的哪一步**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 两块账读不出来（启动参数不足 / 清单头非法）。
    BootArgs,
    /// 那台机器的自述（`Key::dtb`）没领到 / 读不懂。
    Machine,
    /// 死亡道那只组。
    Group,
    /// 整表装配那一趟带来的号（**按服务分的号取自那一台自己的 `died`**，由 `Assembly::assemble` 折出）。
    Assemble(env::Reason),
    /// 监督那一趟。
    Supervise,
    /// 收尾那一趟。
    Doom,
}

impl From<bootstrap::Fail> for Fail {
    fn from(f: bootstrap::Fail) -> Fail {
        match f {
            bootstrap::Fail::BootArgs => Fail::BootArgs,
            bootstrap::Fail::Machine => Fail::Machine,
        }
    }
}

impl Fail {
    fn code(self) -> env::Reason {
        match self {
            Fail::BootArgs => bootstrap::Fail::BootArgs.code(),
            Fail::Machine => bootstrap::Fail::Machine.code(),
            Fail::Assemble(code) => code,
            Fail::Group => control::E_TABLE,
            Fail::Supervise => 8,
            Fail::Doom => 9,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Fail::BootArgs => bootstrap::Fail::BootArgs.text(),
            Fail::Machine => bootstrap::Fail::Machine.text(),
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
        // **收场那一笔有话说**：本域是最后一个域，这一行就是"整机走完了"的记号。
        Ok(()) => programs::Report::note(env::EXIT_OK, "system: done"),
        Err(f) => programs::Report::note(f.code(), f.text()),
    }
}

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办。
fn system() -> Result<(), Fail> {
    // 1. 起手三样：两块账、机器自述、清单（配给与镜像都从它们来；本域就是引导镜像）。
    let boot = bootstrap::take().map_err(Fail::from)?;

    // 2. 这一景起哪些台（**次序由各台声明里的 `after` 算出来**：先起的先就绪，后面的就能向它要东西）。
    let list = scene::programs(&boot.catalog).map_err(|_| Fail::Assemble(E_PROGRAM))?;
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

    // 4. **这一趟走完**（`programs::unit::SCENE` 那一格到点）：**先把 `control` 那一面挂上树**
    //    ——那一刻起那一面才有人待客（`probe-control` 等的就是这一件事，它靠 `after` 里那条
    //    `SCENE` 边排到最后一位）；然后进监督那一趟。
    assembly.mount_control();
    //    监督那一趟：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；**该收了**就下刀，**收讫了**才收场
    //    。
    if !assembly.supervise() {
        return Err(Fail::Doom);
    }
    // 5. 本域退出 = **最后一个域退出** ⇒ 内核收场（shutdown 钩子）⇒ 停机。
    Ok(())
}

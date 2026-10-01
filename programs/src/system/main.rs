#![no_std]
#![no_main]

extern crate alloc;
extern crate programs;

use programs::system::Assembly;
use programs::system::control::{self, E_PROGRAM};
use programs::system::run::bootstrap;
use programs::system::run::scene;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 两块账读不出来（启动参数不足 / 清单头非法）。
    BootArgs,
    /// 那台机器的自述（Key::dtb）没领到 / 读不懂。
    Machine,
    /// 死亡道那只组。
    Group,
    Assemble(env::Reason),
    Supervise,
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
        Ok(()) => programs::Report::note(env::EXIT_OK, "system: done"),
        Err(f) => programs::Report::note(f.code(), f.text()),
    }
}

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办。
fn system() -> Result<(), Fail> {
    let boot = bootstrap::take().map_err(Fail::from)?;

    // 2. 这一景起哪些台（**次序由各台声明里的 `after` 算出来**：先起的先就绪，后面的就能向它要东西）。
    let list = scene::programs(&boot.catalog).map_err(|_| Fail::Assemble(E_PROGRAM))?;
    // **空单**：这一景一台可装配的都没有 ⇒ 报那一格（"有单可装"是下面每一趟的前提）。
    if list.is_empty() {
        return Err(Fail::Assemble(E_PROGRAM));
    }

    // 死亡道跟着这张单铸：要存在信号的那几位一位一条——在 Assembly::new 里。
    let mut assembly = Assembly::new(boot).map_err(|_| Fail::Group)?;

    // 3. 逐条起：**每一台按它自己那份声明装配**（立账 → 建域产线程 → 装通道 → 身份 → 放行等
    //    就绪 → 递配给 → 存在信号 → 树），失败带的是**那一台自己的号**。
    //    都不改变归属），起完就不指着它了——装配者往后只通过板 / 树那两条路与它说话。
    for program in &list {
        assembly.assemble(program).map_err(Fail::Assemble)?;
    }

    assembly.mount_control();
    //    。
    if !assembly.supervise() {
        return Err(Fail::Doom);
    }
    Ok(())
}

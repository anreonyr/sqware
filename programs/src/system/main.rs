#![no_std]
#![no_main]

extern crate alloc;
extern crate programs;

use alloc::boxed::Box;
use alloc::vec::Vec;
use programs::system::Assembly;
use programs::system::control::{self, E_PROGRAM};
use programs::system::run::bootstrap;
use programs::system::run::scene;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 两块账读不出来（启动参数不足 / 清单头非法）
    BootArgs,
    /// 那台机器的自述（Key::dtb）没领到 / 读不懂
    Machine,
    /// 死亡道那只组
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

/// **编排域那一枚的身子**：这台机器上有哪些服务、怎么起、谁死了怎么办
fn system() -> Result<(), Fail> {
    let (mut assembly, list) = prepare()?;

    for program in &list {
        assembly.assemble(program).map_err(Fail::Assemble)?;
    }
    assembly.mount_control();
    if !assembly.supervise() {
        return Err(Fail::Doom);
    }
    Ok(())
}

/// Construction temporaries must leave the fixed Task stack before nested service IPC.
fn prepare() -> Result<(Box<Assembly>, Vec<&'static programs::unit::UnitFile>), Fail> {
    let boot = bootstrap::take().map_err(Fail::from)?;

    // 2. 这一景起哪些台（**次序由各台声明里的 `after` 算出来**：先起的先就绪，后面的就能向它要东西）。
    let list = scene::programs(&boot.catalog).map_err(|_| Fail::Assemble(E_PROGRAM))?;
    // **空单**：这一景一台可装配的都没有 ⇒ 报那一格（"有单可装"是下面每一趟的前提）。
    if list.is_empty() {
        return Err(Fail::Assemble(E_PROGRAM));
    }

    // 死亡道跟着这张单铸：要存在信号的那几位一位一条——在 Assembly::new 里。
    let assembly = Assembly::new(boot).map_err(|_| Fail::Group)?;
    Ok((Box::new(assembly), list))
}

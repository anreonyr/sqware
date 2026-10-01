
use runtime::core::res::dock::Dock;
use runtime::env::mail::PolePie;

use crate::boot::{Accounts, Catalog};
use crate::system::machine::Machine;

/// **起手那一族的死法**：一格 = 死在起手的哪一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 两块账读不出来（启动参数不足 / 清单头非法）。
    BootArgs,
    /// 那台机器的自述（`Key::dtb`）没领到 / 读不懂。
    Machine,
}

impl Fail {
    pub fn code(self) -> env::Reason {
        match self {
            Fail::BootArgs => E_BOOT,
            Fail::Machine => 5,
        }
    }

    pub const fn text(self) -> &'static str {
        match self {
            Fail::BootArgs => "system: boot args unreadable",
            Fail::Machine => "system: no machine",
        }
    }
}

/// 引导那一族共用的号（"两块账读不出来"那一格）。
use crate::unit::system::E_BOOT;

/// **起手要的三样东西**：两块账、那台机器的自述、那块清单。
pub struct Boot {
    /// boot 的两块账：**发货那一趟按坐标取源**（全机 21 枚门闩在本域表里）。
    pub accounts: Accounts,
    /// 本域手里那台机器的自述——单子上那一格写的是**类**，翻成"哪一段区"要有它。
    pub machine: Machine,
    /// 这块字节里**清单与全部镜像都在里头**（本域那张只读视图）。
    pub catalog: Catalog<'static>,
}

pub fn take() -> Result<Boot, Fail> {
    // 两块账：启动参数指的两区（读不出就没得装配）。自述一行：按判别号数它有什么。
    let accounts = Accounts::take().ok_or(Fail::BootArgs)?;
    accounts.report();
    // 清单：账里那整块字节（头已在 `take` 里验过）。
    let catalog = Catalog::of_boot(&accounts).ok_or(Fail::BootArgs)?;
    // 树：本域自己那枚 dtb 门闩——内核给第一域的那一批是 `FETCH|VEST`，开得动。
    let machine = take_machine(&accounts).ok_or(Fail::Machine)?;
    Ok(Boot {
        accounts,
        machine,
        catalog,
    })
}

/// 领树：开门读那段自描述区（**本域那枚门闩，不经任何人**）。
fn take_machine(accounts: &Accounts) -> Option<Machine> {
    let token = accounts.token(env::Key::dtb())?;
    let dock = Dock::open(PolePie::from_token(token)).ok()?;
    Machine::of(dock.view()).ok()
}

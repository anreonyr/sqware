use runtime::core::res::dock::Dock;

use crate::boot::{Accounts, Catalog};
use crate::system::common::machine::Machine;

/// **起手那一族的死法**：一格 = 死在起手的哪一步
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    /// 两块账读不出来（启动参数不足 / 清单头非法）
    BootArgs,
    /// 那台机器的自述（Page::Dtb）没领到 / 读不懂
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

/// 引导那一族共用的号（"两块账读不出来"那一格）
use crate::unit::system::E_BOOT;

/// **起手要的三样东西**：两块账、那台机器的自述、那块清单
pub struct Boot {
    pub accounts: Accounts,
    pub machine: Machine,
    pub catalog: Catalog<'static>,
}

pub fn take() -> Result<Boot, Fail> {
    // 两块账：启动参数指的两区（读不出就没得装配）。自述一行：按判别号数它有什么。
    let accounts = Accounts::take().ok_or(Fail::BootArgs)?;
    accounts.report();
    // 清单：账里那整块字节（头已在 `take` 里验过）。
    let catalog = Catalog::of_boot(&accounts).ok_or(Fail::BootArgs)?;
    let machine = take_machine(&accounts).ok_or(Fail::Machine)?;
    Ok(Boot {
        accounts,
        machine,
        catalog,
    })
}

fn take_machine(accounts: &Accounts) -> Option<Machine> {
    let token = accounts.token(env::Name::Page(env::Page::Dtb))?;
    let dock = Dock::open(token).ok()?;
    Machine::of(dock.view()).ok()
}

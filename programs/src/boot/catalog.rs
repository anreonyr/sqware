//! 按名字挑一台，取它那段字节与特权级。

use env::ledger::manifest;

use super::Accounts;

/// **清单的读面**：按名字挑一台，取它那段字节与特权级
/// 清单里的镜像是**相对这块字节的切片**，故换一张表、换一个 VA 都照样解析得出来
#[derive(Clone, Copy)]
pub struct Catalog<'a> {
    view: &'a [u8],
}

impl<'a> Catalog<'a> {
    /// 拿一块字节当清单。`None` = 清单头非法（条数为零 / 超上限 / 装不下）
    pub fn new(view: &'a [u8]) -> Option<Catalog<'a>> {
        manifest::Entries::new(view)?;
        Some(Catalog { view })
    }

    pub fn of_boot(accounts: &Accounts) -> Option<Catalog<'static>> {
        Catalog::new(accounts.view())
    }

    /// 从清单里挑出这个程序
    pub fn find(&self, want: &str) -> Option<manifest::Entry<'a>> {
        let mut list = self.programs();
        loop {
            let entry = list.next()?;
            let Ok(entry) = entry else { return None };
            if entry.name == want {
                return Some(entry);
            }
        }
    }

    fn programs(&self) -> manifest::Entries<'a> {
        manifest::Entries::new(self.view).expect("清单头已在 new 时验过")
    }
}

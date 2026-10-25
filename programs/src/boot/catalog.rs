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
    /// 校验部署头和全部镜像记录；非法清单不提供部分查找结果。
    pub fn new(view: &'a [u8]) -> Option<Catalog<'a>> {
        for entry in manifest::Entries::new(view)? {
            entry.ok()?;
        }
        Some(Catalog { view })
    }

    pub fn of_boot(accounts: &Accounts) -> Option<Catalog<'static>> {
        Catalog::new(accounts.view())
    }

    pub fn deployment(&self) -> &'a str {
        self.programs().deployment()
    }

    pub fn entry(&self) -> manifest::Entry<'a> {
        self.programs().entry().expect("引导项已在 new 时验过")
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

    /// Iterate the validated manifest records.
    pub fn entries(&self) -> manifest::Entries<'a> {
        self.programs()
    }

    fn programs(&self) -> manifest::Entries<'a> {
        manifest::Entries::new(self.view).expect("部署头已在 new 时验过")
    }
}

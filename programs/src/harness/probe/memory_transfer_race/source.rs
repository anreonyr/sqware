//! 本景那两份私有来源：**迁移来源**与**共享控制页**。
//!
//! 两份都是普通私有分配（属主就是管理端），各自铸成一个**共享根**
//! （`UnsealArgs::Pole { shared: true }`）——于是可以各授一枚子分支给对端，
//! 而对端在**自己那个空间**里把子分支 `open` 成一条压着同一批帧的关系。
//!
//! 两份来源各有各的寿命：控制页到本景收尾才关，迁移来源在被移交动过之后也一直
//! 持到对端读数回来。控制页**不是**迁移范围的一部分（迁移只在来源那两页之间）。
//!
//! 铸根那一手把那份引用**收下**（`create_pole` 消费它），故这里不再 release 它；
//! 移交两端用的那一份引用是另一份——在根窗口上新取的一份，收尾时归还。

use env::AcquireSource;
use env::Allocation;
use env::FlatReference;
use env::MemoryFail;
use env::PAGE_SIZE;
use env::Permission;
use env::PieToken;
use env::Placement;
use env::ReleaseMode;
use env::TaskId;
use env::TeamId;
use env::UnsealArgs;
use env::ViewControl;
use env::VirtAddr;
use execution::memory::Reference;
use execution::memory::View;

use super::super::memory_transfer::fault;

/// 一次 `Attach` 拿到的当前权限：可读可写。
///
/// 这里的 `6` 是 `Access` 在**线上的那几位**（读 | 写），不是 `Permission`
/// （门闩权限位）——与 D1 那一景同一格事实。
const READ_WRITE: u64 = 6;

/// 授给对端的那一份权限：能取能存。
///
/// 与共享根的线上形状同形：两边都没有 `ONLY`（共享根不带独占位）。
const PEER_SUBSET: Permission = Permission::FETCH.union(Permission::STORE);

/// 迁移来源：两页私有分配，第 0 页挂成管理端自己空间里的一条关系。
pub struct Source {
    view: View,
    end: Reference,
    root: PieToken,
    address: usize,
}

impl Source {
    /// 取一份私有来源：前 `mapped` 页挂成本空间的一条关系（起点在 `at`）。
    ///
    /// 引用先取、关系先挂，然后才铸根——铸根会把这份引用与它名下的关系一并收下。
    pub fn mint(pages: usize, at: usize, mapped: usize) -> Source {
        let bytes = pages * PAGE_SIZE;
        let allocation =
            Allocation::new(bytes, PAGE_SIZE).expect("memory-transfer-race: source allocation");
        let source = AcquireSource::Allocation(allocation);
        let reference = Reference::acquire(source, PieToken::NONE)
            .unwrap_or_else(|error| panic!("race: source reference: {:?}", error.source));
        let placement = Placement::exact(VirtAddr::new(at), PAGE_SIZE)
            .expect("memory-transfer-race: source placement");
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                mapped,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .unwrap_or_else(|error| panic!("race: source view: {:?}", error.source));
        let handle = reference
            .handle()
            .expect("memory-transfer-race: source handle");
        let root = env::pie::unseal(UnsealArgs::Pole {
            reference: handle,
            shared: true,
        })
        .unwrap_or_else(|error| panic!("race: source root: {:?}", error.source));
        let end = AcquireSource::Source {
            token: root,
            offset: 0,
            count: pages,
        };
        let end = Reference::acquire(end, PieToken::NONE)
            .unwrap_or_else(|error| panic!("race: source end: {:?}", error.source));
        Source {
            view,
            end,
            root,
            address: at,
        }
    }

    /// 第 `index` 页在本域里的起点。
    pub fn page(&self, index: usize) -> usize {
        self.address + index * PAGE_SIZE
    }

    /// 这一份来源在本任务引用表里的句柄：移交两端各要一枚（同一份就同一枚）。
    pub fn handle(&self) -> FlatReference {
        self.end
            .handle()
            .expect("memory-transfer-race: source end handle")
    }

    /// 把第 `page` 页写成那一页的花纹（头一个字节会走一次真缺页）。
    pub fn fill(&self, page: usize) {
        fault::fill(page, self.page(page));
    }

    /// 把第 `from` 页的承载换进同一份来源的第 `to` 页。
    ///
    /// 这一手就是本景要证的那一次移交；`Ok` = 已结算，`Err` = 那一个失败词汇。
    pub fn shift(&self, from: usize, to: usize) -> Result<(), MemoryFail> {
        let handle = self.handle();
        execution::memory::transfer(handle, from, handle, to).map_err(|error| error.source)
    }

    /// 授一枚子分支给对端：对端凭它在自己那个空间里把这一段 `open` 出来。
    pub fn accord(&self, task: TaskId) -> PieToken {
        env::pie::accord(self.root, task, PEER_SUBSET, env::Mark::NONE)
            .unwrap_or_else(|error| panic!("race: source accord: {:?}", error.source))
    }

    /// 收尾：先关自己那条关系，再交还那份引用，最后把根交回去（子树一起关）。
    pub fn close(&mut self) {
        self.view
            .close()
            .unwrap_or_else(|error| panic!("race: source view close: {:?}", error.source));
        self.end
            .release()
            .unwrap_or_else(|error| panic!("race: source end release: {:?}", error.source));
        env::pie::release(self.root, ReleaseMode::Revoke)
            .unwrap_or_else(|error| panic!("race: source root: {:?}", error.source));
    }
}

/// 共享控制页：一页私有分配，铸成一个共享根，挂在管理端空间的定点上。
pub struct Shared {
    view: View,
    root: PieToken,
}

impl Shared {
    /// 挂上这一页（起点就是 `handshake::VA`）并铸根。
    pub fn mint(at: usize) -> Shared {
        let allocation = Allocation::new(PAGE_SIZE, PAGE_SIZE)
            .expect("memory-transfer-race: control allocation");
        let source = AcquireSource::Allocation(allocation);
        let reference = Reference::acquire(source, PieToken::NONE)
            .unwrap_or_else(|error| panic!("race: control reference: {:?}", error.source));
        let placement = Placement::exact(VirtAddr::new(at), PAGE_SIZE)
            .expect("memory-transfer-race: control placement");
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                1,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .unwrap_or_else(|error| panic!("race: control view: {:?}", error.source));
        let handle = reference
            .handle()
            .expect("memory-transfer-race: control handle");
        let root = env::pie::unseal(UnsealArgs::Pole {
            reference: handle,
            shared: true,
        })
        .unwrap_or_else(|error| panic!("race: control root: {:?}", error.source));
        Shared { view, root }
    }

    /// 授一枚子分支给对端：对端凭它在自己空间里把这一页 `open` 出来。
    pub fn accord(&self, task: TaskId) -> PieToken {
        env::pie::accord(self.root, task, PEER_SUBSET, env::Mark::NONE)
            .unwrap_or_else(|error| panic!("race: control accord: {:?}", error.source))
    }

    /// 收尾：先关自己那条关系，再把根交回去。
    pub fn close(&mut self) {
        self.view
            .close()
            .unwrap_or_else(|error| panic!("race: control close: {:?}", error.source));
        env::pie::release(self.root, ReleaseMode::Revoke)
            .unwrap_or_else(|error| panic!("race: control root: {:?}", error.source));
    }
}

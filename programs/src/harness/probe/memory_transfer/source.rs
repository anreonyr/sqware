//! 一段**私有来源** ＋ 同一空间里那一条 View：三个阶段共用的底座。
//!
//! 来源是私有分配：属主就是发起这次 acquire 的那个任务，故它自己就是这份来源的管理者
//! （`Transfer` 两端的管理判据因此自然成立）。引用一直持着（移出与填回都要用它），
//! 每阶段收尾才显式放下；View 撤下则在引用之前，否则承载会晚一步才归零。

use env::{
    AcquireSource, Allocation, FlatReference, PAGE_SIZE, PieToken, Placement, TeamId, ViewControl,
    ViewId, ViewSelection,
};
use execution::memory::Reference;

/// 一次 `Attach` 给这一段的当前权限：可读可写。
///
/// 这里的 `6` 是 `Access`（`MemoryAccess`）在**线上的那几位**（读 | 写），
/// 不是 `Permission`（门闩权限位）——`execution::memory` 当下就是收这个线上位。
const READ_WRITE: u64 = 6;

pub struct Source {
    reference: Reference,
    id: ViewId,
    address: usize,
}

impl Source {
    /// 取 `pages` 页私有来源，并把前 `mapped` 页挂成本空间的**一条** View。
    pub fn acquire(pages: usize, mapped: usize) -> Source {
        let allocation = Allocation::new(pages * PAGE_SIZE, PAGE_SIZE)
            .expect("memory-transfer: source allocation");
        let reference = Reference::acquire(AcquireSource::Allocation(allocation), PieToken::NONE)
            .unwrap_or_else(|error| panic!("memory-transfer: source acquire: {:?}", error.source));
        let placement = Placement::lowest(PAGE_SIZE).expect("memory-transfer: source placement");
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                mapped,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .unwrap_or_else(|error| panic!("memory-transfer: source attach: {:?}", error.source));
        let id = view.id().expect("memory-transfer: view identity");
        let address = view.address();
        Source {
            reference,
            id,
            address,
        }
    }

    /// 第 `index` 页在这条 View 里的起点。
    pub fn page(&self, index: usize) -> usize {
        self.address + index * PAGE_SIZE
    }

    /// 这条 View 的身份：等待位就是按它与空间的 ASID 定的。
    pub fn id(&self) -> ViewId {
        self.id
    }

    /// 这份来源在本任务引用表里的句柄：跨**两份**来源的移交两端各要一枚。
    pub fn handle(&self) -> FlatReference {
        self.reference
            .handle()
            .expect("memory-transfer: source handle")
    }

    /// 把第 `from` 页的承载换进第 `to` 页（同一份来源里的空槽）。
    pub fn shift(&self, from: usize, to: usize) {
        let handle = self
            .reference
            .handle()
            .expect("memory-transfer: source handle");
        execution::memory::transfer(handle, from, handle, to).unwrap_or_else(|error| {
            panic!("memory-transfer: shift {from}->{to}: {:?}", error.source)
        });
    }

    /// 只把第 `index` 页那一段关系撤下来（同一段 View 的其它页不动）。
    pub fn withdraw(&self, index: usize) {
        let selection = ViewSelection::pages(index, 1).expect("memory-transfer: page selection");
        execution::memory::detach(TeamId::new(0), self.id, selection).unwrap_or_else(|error| {
            panic!("memory-transfer: withdraw page {index}: {:?}", error.source)
        });
    }

    /// 收尾：撤下这一条 View，再放下那份引用（承载随之归零）。
    pub fn close(&mut self) {
        execution::memory::detach(TeamId::new(0), self.id, ViewSelection::Whole)
            .unwrap_or_else(|error| panic!("memory-transfer: close view: {:?}", error.source));
        self.reference
            .release()
            .unwrap_or_else(|error| panic!("memory-transfer: release: {:?}", error.source));
    }
}

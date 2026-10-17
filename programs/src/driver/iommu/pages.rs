//! 硬件可达页：一段经来源授权、由独立保留强持的物理页窗口。
//!
//! 发布给设备的是 `traverse` 出口给出的**真实物理页起址**，不是本域地址；
//! 本地那一条 View 只用来把 DDT / 页表 / 队列 / 完成记录填好。

use alloc::vec::Vec;
use env::{AcquireSource, PAGE_SIZE, PieToken, Placement, ReferenceRange, TeamId, ViewControl};
use execution::memory::{Reference, View, traverse};

/// 线上那几位：读 | 写。
const READ_WRITE: u64 = 6;

pub struct Pages {
    pole: PieToken,
    reference: Reference,
    view: View,
    physical: Vec<u64>,
}

impl Pages {
    /// 取 `count` 页：先立一份匿名来源（Pole），再从它取一段**带保留**的窗口。
    ///
    /// 保留那一份是这一段的真实强持，`traverse` 与设备地址发布都以它为准。
    pub fn take(retention: PieToken, count: usize) -> Result<Self, &'static str> {
        let pole =
            resource::raw::pole(count * PAGE_SIZE, true).map_err(|_| "iommu: page pole")?;
        let reference = Reference::acquire(
            AcquireSource::Source {
                token: pole,
                offset: 0,
                count,
            },
            retention,
        )
        .map_err(|_| "iommu: retained window")?;
        let placement = Placement::lowest(PAGE_SIZE).ok_or("iommu: page placement")?;
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                count,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .map_err(|_| "iommu: page view")?;
        let handle = reference.handle().map_err(|_| "iommu: page handle")?;
        let range = ReferenceRange::new(handle, 0, count).ok_or("iommu: page range")?;
        let mut physical = Vec::new();
        physical
            .try_reserve_exact(count)
            .map_err(|_| "iommu: page table")?;
        physical.resize(count, 0);
        traverse(retention, range, &mut physical).map_err(|_| "iommu: physical export")?;
        // 页是新取的匿名页：先清零，免得把上一轮残留当内容。
        // SAFETY: 这一段是本域刚挂上的可读写 View，`count` 页连成一片。
        unsafe { core::ptr::write_bytes(view.address() as *mut u8, 0, count * PAGE_SIZE) };
        Ok(Self {
            pole,
            reference,
            view,
            physical,
        })
    }

    /// 这一段所在的**匿名来源**（本域持有）：另起一份保留、或向别的任务
    /// 授一段窗口（`accord_window`）都以它为准。
    pub fn source(&self) -> PieToken {
        self.pole
    }

    /// 这一段有多少页。
    pub fn count(&self) -> usize {
        self.physical.len()
    }

    /// 第 `index` 页的物理起址（设备要看到的那个数）。
    pub fn physical(&self, index: usize) -> usize {
        self.physical[index] as usize
    }

    /// 第 `index` 页在本域、偏移 `offset` 处的地址。
    pub fn at(&self, index: usize, offset: usize) -> *mut u8 {
        (self.view.address() + index * PAGE_SIZE + offset) as *mut u8
    }

    /// 第 `index` 页、偏移 `at` 处写一个字节（本域那一份视图）。
    pub fn put8(&self, index: usize, at: usize, value: u8) {
        // SAFETY: 这一段是本域已映射的可写 View，`index`/`at` 落在窗内。
        unsafe { core::ptr::write_volatile(self.at(index, at), value) };
    }

    pub fn put16(&self, index: usize, at: usize, value: u16) {
        // SAFETY: 同 `put8`，偏移按 2 字节格取。
        unsafe { core::ptr::write_volatile(self.at(index, at) as *mut u16, value) };
    }

    pub fn put32(&self, index: usize, at: usize, value: u32) {
        // SAFETY: 同 `put8`，偏移按 4 字节格取。
        unsafe { core::ptr::write_volatile(self.at(index, at) as *mut u32, value) };
    }

    pub fn put64(&self, index: usize, at: usize, value: u64) {
        // SAFETY: 同 `put8`，偏移按 8 字节格取。
        unsafe { core::ptr::write_volatile(self.at(index, at) as *mut u64, value) };
    }

    pub fn get8(&self, index: usize, at: usize) -> u8 {
        // SAFETY: 同 `put8`。
        unsafe { core::ptr::read_volatile(self.at(index, at)) }
    }

    pub fn get16(&self, index: usize, at: usize) -> u16 {
        // SAFETY: 同 `put8`，偏移按 2 字节格取。
        unsafe { core::ptr::read_volatile(self.at(index, at) as *const u16) }
    }

    pub fn get32(&self, index: usize, at: usize) -> u32 {
        // SAFETY: 同 `put8`，偏移按 4 字节格取。
        unsafe { core::ptr::read_volatile(self.at(index, at) as *const u32) }
    }

    /// 把一整页铺成同一个字节。
    pub fn fill(&self, index: usize, byte: u8) {
        // SAFETY: 整页都在本域这一段可写 View 里。
        unsafe { core::ptr::write_bytes(self.at(index, 0), byte, PAGE_SIZE) };
    }

    /// 显式结清：先撤本域的图，再放下保留，最后放下匿名来源。
    pub fn close(mut self) -> Result<(), &'static str> {
        self.view.close().map_err(|_| "iommu: view close")?;
        self.reference
            .release()
            .map_err(|_| "iommu: page release")?;
        resource::raw::release(self.pole).map_err(|_| "iommu: page source")
    }
}

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
    reference: Reference,
    view: View,
    physical: Vec<u64>,
}

impl Pages {
    /// 取 `count` 页：先立一份匿名来源（Pole），再从它取一段**带保留**的窗口。
    ///
    /// 保留那一份是这一段的真实强持，`traverse` 与设备地址发布都以它为准。
    pub fn take(retention: PieToken, count: usize) -> Result<Self, &'static str> {
        let pole = resource::raw::pole(count * PAGE_SIZE, true).map_err(|_| "iommu-probe: page pole")?;
        let reference = Reference::acquire(
            AcquireSource::Source {
                token: pole,
                offset: 0,
                count,
            },
            retention,
        )
        .map_err(|_| "iommu-probe: retained window")?;
        let placement = Placement::lowest(PAGE_SIZE).ok_or("iommu-probe: page placement")?;
        let view = reference
            .attach(
                TeamId::new(0),
                0,
                count,
                placement,
                READ_WRITE,
                ViewControl::Caller,
            )
            .map_err(|_| "iommu-probe: page view")?;
        let handle = reference.handle().map_err(|_| "iommu-probe: page handle")?;
        let range = ReferenceRange::new(handle, 0, count).ok_or("iommu-probe: page range")?;
        let mut physical = Vec::new();
        physical.try_reserve_exact(count).map_err(|_| "iommu-probe: page table")?;
        physical.resize(count, 0);
        traverse(retention, range, &mut physical).map_err(|_| "iommu-probe: physical export")?;
        // 页是新取的匿名页：先清零，免得把上一轮残留当内容。
        // SAFETY: 这一段是本域刚挂上的可读写 View，`count` 页连成一片。
        unsafe { core::ptr::write_bytes(view.address() as *mut u8, 0, count * PAGE_SIZE) };
        Ok(Self {
            reference,
            view,
            physical,
        })
    }

    /// 第 `index` 页的物理起址（设备要看到的那个数）。
    pub fn physical(&self, index: usize) -> usize {
        self.physical[index] as usize
    }

    /// 第 `index` 页在本域、偏移 `offset` 处的地址。
    pub fn at(&self, index: usize, offset: usize) -> *mut u8 {
        (self.view.address() + index * PAGE_SIZE + offset) as *mut u8
    }

    /// 显式结清：先撤本域的图，再放下保留。
    pub fn close(mut self) -> Result<(), &'static str> {
        self.view.close().map_err(|_| "iommu-probe: view close")?;
        self.reference.release().map_err(|_| "iommu-probe: page release")
    }
}

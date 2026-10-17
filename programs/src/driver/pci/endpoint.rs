//! 这一台 PCI 端点：它在 PCI 上的号（IOMMU 用它认设备）＋**交给驱动的那一段** BAR。
//!
//! `Endpoint` 持有 ECAM 映射的地址：那段映射必须活到本对象的最后一次访问之后。

use super::ecam::{self, Ecam};

/// 一台 BAR 已落位、内存译码已打开的端点：发起权由 `grant_master` 另授。
#[derive(Clone, Copy)]
pub struct Endpoint {
    scan: Ecam,
    address: u64,
    size: u64,
}

impl Endpoint {
    /// 把 `ecam` 那一个功能的第 `index` 根内存 BAR 落进 `window`（窗口起址, 字节数），
    /// 并打开内存译码；bus-master 那一位不动。
    ///
    /// 落位按 BAR 的直径对齐从窗口里取一段，放不下当场拒。
    pub fn configure(ecam: &Ecam, index: u8, window: (u64, u64)) -> Result<Self, &'static str> {
        let (size, wide) = ecam.bar_size(index).ok_or("pci: endpoint bar")?;
        if !wide || size == 0 {
            return Err("pci: endpoint bar width");
        }
        let address = window.0.div_ceil(size) * size;
        let end = address.checked_add(size).ok_or("pci: bar placement")?;
        if end > window.0 + window.1 {
            return Err("pci: bar placement");
        }
        ecam.place_bar(index, address, wide);
        ecam.command_or(ecam::COMMAND_MEMORY);
        Ok(Self {
            scan: *ecam,
            address,
            size,
        })
    }

    /// 置上 bus-master 并回读那一位：`0` = 此刻没有发起权。
    pub fn grant_master(&self) -> u16 {
        self.scan.command_or(ecam::COMMAND_MASTER);
        self.master()
    }

    /// 清掉 bus-master 并回读那一位：`0` = 发起权已撤。
    pub fn revoke_master(&self) -> u16 {
        self.scan.command_and(ecam::COMMAND_MASTER);
        self.master()
    }

    /// 只读 bus-master 那一位。
    pub fn master(&self) -> u16 {
        self.scan.read16(ecam::COMMAND) & ecam::COMMAND_MASTER
    }

    /// 这一台在 IOMMU 眼里的号：设备目录表按它索引。
    pub const fn requester(&self) -> u16 {
        self.scan.requester()
    }

    /// 那一段 BAR 的**绝对物理**起址（落在本来那段 PCI 窗口里）。
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn size(&self) -> u64 {
        self.size
    }
}

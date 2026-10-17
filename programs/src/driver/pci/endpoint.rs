//! 这一台 PCI 端点：它在 PCI 上的号（IOMMU 用它认设备）＋**交给驱动的那一段** BAR。

use super::ecam::{self, Ecam};

/// 一台已经把 BAR 落好位的端点。`address`/`size` 就是“精确 BAR”那两个数。
pub struct Endpoint {
    requester: u16,
    address: u64,
    size: u64,
}

impl Endpoint {
    /// 把 `ecam` 那一个功能的第 `index` 根内存 BAR 落进 `window`（窗口起址, 字节数）。
    ///
    /// 落位从窗口里按 BAR 的直径对齐取一段；放不下当场拒（不硬塞、不另找地方）。
    /// 落位之后打开内存空间与 bus master：这是后端真能发起 DMA 的前提。
    pub fn claim(ecam: &Ecam, index: u8, window: (u64, u64)) -> Result<Self, &'static str> {
        let (size, wide) = ecam.bar_size(index).ok_or("pci: endpoint bar")?;
        if !wide || size == 0 {
            return Err("pci: endpoint bar width");
        }
        let address = window.0.div_ceil(size) * size;
        let end = address
            .checked_add(size)
            .ok_or("pci: bar placement")?;
        if end > window.0 + window.1 {
            return Err("pci: bar placement");
        }
        ecam.place_bar(index, address, wide);
        ecam.command_or(ecam::COMMAND_MEMORY | ecam::COMMAND_MASTER);
        Ok(Self {
            requester: ecam.requester(),
            address,
            size,
        })
    }

    /// 这一台在 IOMMU 眼里的号：设备目录表按它索引。
    pub const fn requester(&self) -> u16 {
        self.requester
    }

    /// 那一段 BAR 的**绝对物理**起址（落在本来那段 PCI 窗口里）。
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub const fn size(&self) -> u64 {
        self.size
    }
}

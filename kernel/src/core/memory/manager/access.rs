use bitflags::bitflags;

use super::entry::PteFlags;

bitflags! {
    /// 映射权限的逻辑三位：只讲读写执行。
    ///
    /// 线上对应 PTE 的 R/W/X，位于 `MemoryCall` 位 1/2/3；U/G/V/A/D 由
    /// `pte_policy` 与 `mapping_policy` 另加。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Access: u64 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXECUTE = 1 << 2;
    }
}

/// 线上三位的掩码：`MemoryCall` 的 flags 里只有这三位合法。
const WIRE_MASK: u64 = 0b1110;

impl Access {
    /// 线上三位折成本地三位；多一位都拒。
    ///
    /// 只做换算，不带授权口径：`Mmap` 与 `Mprotect` 各自在调用点判该拒什么。
    pub fn from_wire(flags: u64) -> Option<Self> {
        if flags & !WIRE_MASK != 0 {
            return None;
        }
        Self::from_bits(flags >> 1)
    }

    pub(super) fn to_pte(self) -> PteFlags {
        PteFlags::from_bits_truncate(self.bits() << 1)
    }
}

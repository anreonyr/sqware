use bitflags::bitflags;

use crate::core::memory::nest::table::PteFlags;

bitflags! {
    /// 映射权限的逻辑三位：只讲读写执行。
    ///
    /// 线上对应 PTE 的 R/W/X，位于 `MemoryCall` 位 1/2/3；U/G/V/A/D 属于
    /// 独立的架构属性，由挂载时的政策给出。
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

    /// 页表里的 RWX 三位折成逻辑三位。
    pub fn from_pte(flags: PteFlags) -> Self {
        Self::from_bits_truncate(flags.bits() >> 1)
    }

    pub(crate) fn to_pte(self) -> PteFlags {
        PteFlags::from_bits_truncate(self.bits() << 1)
    }

    /// 合法叶子权限：RWX 非空，且写必须有读。
    pub(crate) fn is_leaf_set(self) -> bool {
        !self.is_empty() && !(self.contains(Self::WRITE) && !self.contains(Self::READ))
    }
}

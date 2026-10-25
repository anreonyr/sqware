//! 本台声明要的两样：保留授权那一叶（既有租约交付）与 IOMMU 寄存器那一段窗。

use crate::unit::Binding;
use env::{Mark, Permission};

/// 本实例的保留授权：既有 `memory/retention` 集合里那一叶。
pub const RETENTION: Binding = Binding {
    source: system_api::supply::Key::of("system/initrd"),
    resource: system_api::supply::Key::of("memory/retention"),
    slot: Mark::of("iommu/retention"),
    permission: Permission::FETCH,
};

/// IOMMU 寄存器那一段（读写 ＋ 跟源）。
pub const REGION: Binding = Binding {
    source: system_api::supply::Key::of("system/initrd"),
    resource: system_api::supply::Key::of("platform/iommu"),
    slot: Mark::of("iommu/region"),
    permission: Permission::FETCH,
};

/// 寄存器窗交进来时的形态。
pub const REGION_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);

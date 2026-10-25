//! 本台声明要的那几扇受限平台窗（装配方登记、定向交付）。
//!
//! 坐标只在本声明包定义一次；runtime 与 composition 都引用这里。

use crate::unit::Binding;
use env::{Mark, Permission};

/// ECAM 那一段（读写 ＋ 跟源）。
pub const ECAM_BINDING: Binding = Binding {
    source: system_api::supply::Key::of("system/initrd"),
    resource: system_api::supply::Key::of("platform/ecam"),
    slot: Mark::of("pci/ecam"),
    permission: Permission::FETCH,
};

/// PCI aperture 那一段（读写、可再授出、跟源）。
pub const APERTURE_BINDING: Binding = Binding {
    source: system_api::supply::Key::of("system/initrd"),
    resource: system_api::supply::Key::of("platform/aperture"),
    slot: Mark::of("pci/aperture"),
    permission: Permission::FETCH,
};

/// 两扇窗交进来时的形态。
pub const ECAM_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);
pub const APERTURE_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::VEST)
    .union(Permission::ONLY);

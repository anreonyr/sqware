//! PCI 驱动那一台的装配声明（身子在 `serve.rs`，入口在 `main.rs`）。
//!
//! 它是**常驻**（Resident）服务：由装配表静态起、由 System 收；硬件那一侧的事实
//! （枚举、BAR 落位、MASTER）都在它自己的 Task/Space 里。两扇平台窗由声明供给定向交到
//! 本台，不经普通设备库存。

use crate::unit::{Dependency, Ending, Identity, Ready, UnitFile};

#[path = "marks.rs"]
mod marks;

#[path = "bindings.rs"]
mod bindings;

pub static PROGRAM: UnitFile = UnitFile {
    name: "pci",
    loader: crate::unit::LoaderConfig::supervisor("pci"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "pci",
            scope: crate::unit::operator::Scope::Names(&["entry"]),
            ops: system_api::operator::Ops::BIND,
            permit: Some(system_api::operator::Permit::Public),
            source_mark: Some(marks::ENTRY.get()),
            bindings: &[],
        }, crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "pci",
            scope: crate::unit::operator::Scope::Names(&["config"]),
            ops: system_api::operator::Ops::BIND,
            permit: Some(system_api::operator::Permit::Public),
            source_mark: Some(marks::CONFIG.get()),
            bindings: &[],
        }],
    },
    identity: Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[
            Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
            Dependency::Unit("hub"),
        ]),
        ready: Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),
        construction: &[],
        retirement: &[crate::unit::Retirement {
            provider: "hub",
            namespace: 0,
            face: "retire",
        }],
    },
    supply: crate::unit::SupplyConfig {
        bindings: &[bindings::ECAM_BINDING, bindings::APERTURE_BINDING],
    },
};

//! 设备恢复那一台的装配声明（身子在 `serve.rs`，入口在 `main.rs`）。
//!
//! 它是**独立常驻**（Resident）的设备恢复方：不在 α 的 Team 子树里，一笔责任结清
//! 不要求它退出；它自己没有任何声明供给（精确 BAR 由 PCI 在协调入口的帧里定向交来）。

use crate::unit::{Dependency, Ending, Identity, Ready, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    name: "recovery",
    loader: crate::unit::LoaderConfig::supervisor("recovery"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "recovery",
            scope: crate::unit::operator::Scope::Subtree,
            ops: system_api::operator::Ops::PART
                .union(system_api::operator::Ops::UNPART)
                .union(system_api::operator::Ops::BIND)
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: Some(system_api::operator::Permit::Public),
            source_mark: None,
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
            Dependency::Unit("pci"),
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
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

//! IOMMU 驱动那一台的装配声明（身子在 `serve.rs`，入口在 `main.rs`）。
//!
//! 生产只留这一张镜像：它是**常驻**服务，由装配表静态起；两样声明供给（保留叶与
//! 寄存器窗）定向交到本台。起手故障、平台暂不确认那些场景由 Harness 编排，
//! 不往这一台塞景码。

use crate::unit::{Dependency, Ending, Identity, Ready, UnitFile};

#[path = "bindings.rs"]
mod bindings;

pub static PROGRAM: UnitFile = UnitFile {
    name: "iommu",
    loader: crate::unit::LoaderConfig::supervisor("iommu"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "iommu",
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
        bindings: &[bindings::RETENTION, bindings::REGION],
    },
};

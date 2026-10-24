//! Resident device Driver with a declared coordination endpoint.
//! It remains outside the execution Team and acquires exact BAR grants from PCI.

use crate::unit::{Dependency, Ending, Identity, Ready, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    name: "recovery",
    loader: crate::unit::LoaderConfig::supervisor("recovery"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "device",
            scope: crate::unit::operator::Scope::Names(&["coord"]),
            ops: system_api::operator::Ops::BIND,
            permit: Some(system_api::operator::Permit::Public),
            source_mark: Some(hub_api::coordination::call::ENTRY.get()),
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
            Dependency::Unit("iommu"),
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

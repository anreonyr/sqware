//! 这一台的装配声明（身子在本目录的 main.rs；"哪几台进哪张镜像"见
//! :catalog 的 `PROGRAMS`

use crate::unit::{Ending, UnitFile};

/// 会死的持有者：落一块**声明归自己**的门牌然后直接死，好让下一台接手
pub static PROBE_LEASE: UnitFile = UnitFile {
    name: "probe-lease",
    loader: crate::unit::LoaderConfig::user("probe-lease"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "fixtures",
            scope: crate::unit::operator::Scope::Names(&["lease"]),
            ops: system_api::operator::Ops::BIND
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: None,
            source_mark: None,
            bindings: &[],
        }],
    },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Operator,
        )]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Transient),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig {
        bindings: &[crate::unit::Binding {
            source: system_api::supply::Key::of("system/initrd"),
            resource: system_api::supply::Key::of("memory/retention"),
            slot: env::Mark::of("probe-lease/retention"),
            permission: env::Permission::FETCH,
        }],
    },
};

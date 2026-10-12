//! terminal::program — **控制台那一台**（`prog-terminal`）的装配声明。
//! **U 态**（最小特权）：只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门。

use crate::unit::{Died, Ending, UnitFile};

/// 它死在起手哪一步。
pub const E_TERMINAL: Died = 24;

pub static PROGRAM: UnitFile = UnitFile {
    name: "terminal",
    loader: crate::unit::LoaderConfig::user("terminal"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "terminal",
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
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("uart"),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

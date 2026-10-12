//! 串口驱动（prog-uart）的装配声明。
//! **U 态**：持有 `serial@10000000`（banner 里那张 PMP 是 S/U (R,W)），把"收到字节就拉线"打开。

use crate::unit::{Died, Ending, UnitFile};

/// 它死在起手 / 常驻哪一步
pub const E_UART: Died = 9;

pub static PROGRAM: UnitFile = UnitFile {
    name: "uart",
    loader: crate::unit::LoaderConfig::user("uart"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "uart",
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
            crate::unit::Dependency::Unit("hub"),
            crate::unit::Dependency::Unit("router"),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

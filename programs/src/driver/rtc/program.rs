//! 实时钟驱动（prog-rtc）的装配声明。
//! **U 态**：持有 `rtc@101000`（11 号线），武装闹钟、到点自己拉线；客人定的闹钟到点就清掉
//! 那一格、把"那一声"推回去。它是"抽象等第二个实例"的那个第二例。

use crate::unit::{Died, Ending, UnitFile};

/// 它死在起手 / 常驻哪一步
pub const E_RTC: Died = 12;

pub static PROGRAM: UnitFile = UnitFile {
    name: "rtc",
    loader: crate::unit::LoaderConfig::user("rtc"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            path: "",
            scope: crate::unit::operator::Scope::Names(&["rtc"]),
            ops: system_api::operator::Ops::BIND
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: Some(system_api::operator::Permit::Public),
            source_mark: None,
            bindings: &[crate::unit::operator::Binding {
                name: "rtc",
                mark: router_api::ENTRY_MARK,
                view: env::Access::STORE,
                permit: system_api::operator::Permit::Public,
            }],
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

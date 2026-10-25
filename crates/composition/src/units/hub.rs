//! 设备账（prog-hub）的装配声明。
//! **U 态**：它不碰寄存器、不碰中断——只读设备树（把坐标补成"名 / 类 / 线"）、立账、落
//! `/dev/<类>/<名>`、答三面；组织资格由 Identity 管理，有效身份须由可信 Control 安装。

use crate::unit::{Ending, UnitFile};
use hub_api::declaration::{INVENTORY_MARK, READY_MARK};

pub static PROGRAM: UnitFile = UnitFile {
    name: "hub",
    loader: crate::unit::LoaderConfig::user("hub"),
    operator: crate::unit::OperatorConfig {
        namespace: &[
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "hub",
                scope: crate::unit::operator::Scope::Names(&["bond", "list", "claim", "retire"]),
                ops: system_api::operator::Ops::BIND
                    .union(system_api::operator::Ops::UNBIND)
                    .union(system_api::operator::Ops::FIND),
                permit: Some(system_api::operator::Permit::Public),
                source_mark: None,
                bindings: &[],
            },
            crate::unit::operator::Namespace {
                key: system_api::namespace::DEVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "",
                scope: crate::unit::operator::Scope::Subtree,
                ops: system_api::operator::Ops::PART
                    .union(system_api::operator::Ops::UNPART)
                    .union(system_api::operator::Ops::BIND)
                    .union(system_api::operator::Ops::UNBIND)
                    .union(system_api::operator::Ops::FIND),
                permit: None,
                source_mark: None,
                bindings: &[],
            },
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "hub",
                scope: crate::unit::operator::Scope::Names(&["dma"]),
                ops: system_api::operator::Ops::BIND,
                permit: Some(system_api::operator::Permit::Public),
                source_mark: Some(hub_api::Grant::Dma.mark().get()),
                bindings: &[],
            },
        ],
    },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[
            system_api::identity::Grant::Found,
            system_api::identity::Grant::Admit,
            system_api::identity::Grant::Expel,
            system_api::identity::Grant::Activate,
        ],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
        retirement: &[crate::unit::Retirement {
            provider: "hub",
            namespace: 0,
            face: "retire",
        }],
    },
    supply: crate::unit::SupplyConfig {
        bindings: &[system_api::supply::Binding {
            source: system_api::supply::Key::of("system/initrd"),
            resource: system_api::supply::Key::of("machine/inventory"),
            slot: INVENTORY_MARK,
            permission: env::Permission::FETCH,
        }],
    },
};

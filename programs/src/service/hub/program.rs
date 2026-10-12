//! 设备账（prog-hub）的装配声明。
//! **U 态**：它不碰寄存器、不碰中断——只读设备树（把坐标补成"名 / 类 / 线"）、立账、落
//! `/dev/<类>/<名>`、答三面；组织资格由 Identity 管理，有效身份须由可信 Control 安装。

use crate::unit::{Died, Ending, UnitFile};

pub const E_HUB: Died = 28;

pub const CHANNEL: &str = "hub";

/// **末尾**铸一枚刻它的孔交回装配者，那一刻它才继续往下起别人
pub const READY: &str = "hub-ready";
/// Hub 的私有启动角色，保持与 Machine setup 的名称对应。
pub const READY_MARK: env::Mark = env::Mark::of(READY);
pub const CHANNEL_MARK: env::Mark = env::Mark::of(CHANNEL);
pub const INVENTORY_MARK: env::Mark = env::Mark::of("hub/machine-inventory");

pub static PROGRAM: UnitFile = UnitFile {
    name: "hub",
    loader: crate::unit::LoaderConfig::user("hub"),
    operator: crate::unit::OperatorConfig {
        namespace: &[
            crate::unit::operator::Namespace {
                key: system_api::namespace::SERVICE,
                placement: crate::unit::operator::Placement::Shared,
                path: "hub",
                scope: crate::unit::operator::Scope::Subtree,
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
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
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

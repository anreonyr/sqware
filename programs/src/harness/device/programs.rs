//! 场景那一侧六台的装配声明（身子在 `work.rs` / `control.rs`，入口在各自 `main.rs`）。
//!
//! 三张 α 镜像（正常／发布前中止／发布后中止）与四张控制器镜像（正常／故障／发布前中止／
//! 发布后中止）：同一身子、不同镜像，场景差异是编译期常量，不进任何生产协议。

use crate::unit::{Dependency, Ending, Identity, Ready, UnitFile};

/// α 与控制器共用一段命名空间声明（服务名下各自一格）。
const fn namespace(path: &'static str) -> crate::unit::operator::Namespace {
    crate::unit::operator::Namespace {
        key: system_api::namespace::SERVICE,
        placement: crate::unit::operator::Placement::Shared,
        path,
        scope: crate::unit::operator::Scope::Subtree,
        ops: system_api::operator::Ops::PART
            .union(system_api::operator::Ops::UNPART)
            .union(system_api::operator::Ops::BIND)
            .union(system_api::operator::Ops::UNBIND)
            .union(system_api::operator::Ops::FIND),
        permit: Some(system_api::operator::Permit::Public),
        source_mark: None,
        bindings: &[],
    }
}

/// 设备那一轴的只读范围（认领走既有设备门准入：类联盟 permit 在门牌上）。
const fn device_namespace() -> crate::unit::operator::Namespace {
    crate::unit::operator::Namespace {
        key: system_api::namespace::DEVICE,
        placement: crate::unit::operator::Placement::Shared,
        path: "",
        scope: crate::unit::operator::Scope::Subtree,
        ops: system_api::operator::Ops::FIND,
        permit: None,
        source_mark: None,
        bindings: &[],
    }
}

/// α 与控制器共用的那几段命名空间声明（服务名下各自一格 ＋ 设备那一轴只读）。
const WORK_NAMESPACE: [crate::unit::operator::Namespace; 2] =
    [namespace("device-work"), device_namespace()];
const CONTROL_NAMESPACE: [crate::unit::operator::Namespace; 2] =
    [namespace("device-ctl"), device_namespace()];

/// 控制器的受限构造授权：三张 α 镜像，**owner 恒为 Hub**（停止与核销归 Hub）。
const CONTROL_CONSTRUCTION: [crate::unit::Construction; 1] =
    [crate::unit::Construction::unit_owned_by(
        "device-work",
        "hub",
    )];
const BEFORE_CONSTRUCTION: [crate::unit::Construction; 1] =
    [crate::unit::Construction::unit_owned_by(
        "device-work-before",
        "hub",
    )];
const AFTER_CONSTRUCTION: [crate::unit::Construction; 1] =
    [crate::unit::Construction::unit_owned_by(
        "device-work-after",
        "hub",
    )];

/// α 的三张镜像共用的声明（只有名字与镜像不同）。
const fn work(name: &'static str, image: &'static str) -> UnitFile {
    UnitFile {
        name,
        loader: crate::unit::LoaderConfig::supervisor(image),
        operator: crate::unit::OperatorConfig {
            namespace: &WORK_NAMESPACE,
        },
        identity: Identity::DEFAULT,
        control: crate::unit::ControlConfig {
            after: Some(&[
                Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
                Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
                Dependency::Unit("hub"),
                Dependency::Unit("pci"),
                Dependency::Unit("iommu"),
                Dependency::Unit("recovery"),
            ]),
            ready: Ready::OnSpawn,
            ending: Some(Ending::Transient),
            construction: &[],
        },
        supply: crate::unit::SupplyConfig { bindings: &[] },
    }
}

/// 控制器的三张镜像共用的声明。
///
/// 它持**受限构造**授权：把执行实例（三张 α 镜像）以 **Hub 为 owner** 建出来，
/// 再报给 Hub 放行；停止与核销也归 Hub（实例面按真实 owner 核）。
const fn control(
    name: &'static str,
    image: &'static str,
    construction: &'static [crate::unit::Construction],
) -> UnitFile {
    UnitFile {
        name,
        loader: crate::unit::LoaderConfig::supervisor(image),
        operator: crate::unit::OperatorConfig {
            namespace: &CONTROL_NAMESPACE,
        },
        identity: Identity::DEFAULT,
        control: crate::unit::ControlConfig {
            after: Some(&[
                Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
                Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
                Dependency::Unit("hub"),
                Dependency::Unit("pci"),
                Dependency::Unit("iommu"),
                Dependency::Unit("recovery"),
            ]),
            ready: Ready::OnSpawn,
            ending: Some(Ending::Transient),
            construction,
        },
        supply: crate::unit::SupplyConfig { bindings: &[] },
    }
}

pub static WORK: UnitFile = work("device-work", "device-work");
pub static WORK_BEFORE: UnitFile = work("device-work-before", "device-work-before");
pub static WORK_AFTER: UnitFile = work("device-work-after", "device-work-after");
pub static CONTROL: UnitFile = control("device-ctl", "device-ctl", &CONTROL_CONSTRUCTION);
pub static CONTROL_RUIN: UnitFile = control(
    "device-ctl-ruin",
    "device-ctl-ruin",
    &CONTROL_CONSTRUCTION,
);
pub static CONTROL_FORCED: UnitFile = control(
    "device-ctl-forced",
    "device-ctl-forced",
    &CONTROL_CONSTRUCTION,
);
pub static CONTROL_SOAK: UnitFile = control(
    "device-ctl-soak",
    "device-ctl-soak",
    &CONTROL_CONSTRUCTION,
);
pub static CONTROL_SOAK_LONG: UnitFile = {
    let mut unit = control("device-ctl-soak-long", "device-ctl-soak", &CONTROL_CONSTRUCTION);
    unit.loader.args = &[1024];
    unit
};
pub static CONTROL_CONFIRMATION: UnitFile = control(
    "device-ctl-confirmation",
    "device-ctl-confirmation",
    &CONTROL_CONSTRUCTION,
);
pub static CONTROL_FAULT: UnitFile = control(
    "device-ctl-fault",
    "device-ctl-fault",
    &CONTROL_CONSTRUCTION,
);
pub static CONTROL_BEFORE: UnitFile = control(
    "device-ctl-before",
    "device-ctl-before",
    &BEFORE_CONSTRUCTION,
);
pub static CONTROL_AFTER: UnitFile =
    control("device-ctl-after", "device-ctl-after", &AFTER_CONSTRUCTION);

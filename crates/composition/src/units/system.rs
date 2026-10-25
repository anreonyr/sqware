//! 编排域（prog-system）自己的装配声明。
//! 它是**这一景的引导镜像**（**不在装配单上**：`after: None`）：起手自己读 boot 的两块账，此后按
//! 各台声明里的 `after` **算出来的次序**依次起服务（unit::order_scene）。

use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "system",
    loader: crate::unit::LoaderConfig::supervisor("system"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

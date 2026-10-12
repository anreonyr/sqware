use crate::unit::{Ending, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    name: "probe-terminal",
    loader: crate::unit::LoaderConfig::user("probe-terminal"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("terminal"),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Transient),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

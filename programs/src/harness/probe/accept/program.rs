use crate::unit::UnitFile;

pub static PROGRAM: UnitFile = UnitFile {
    name: "accept-system",
    loader: crate::unit::LoaderConfig::supervisor("accept-system"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

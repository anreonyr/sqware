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
        supervision: None,
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

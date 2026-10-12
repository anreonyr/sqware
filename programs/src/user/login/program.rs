use crate::unit::{Ending, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    name: "login",
    loader: crate::unit::LoaderConfig::user("login"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Unit("terminal"),
            crate::unit::Dependency::Unit("account"),
            crate::unit::Dependency::Target(crate::unit::SCENE),
        ]),
        ready: crate::unit::Ready::OnSpawn,
        ending: Some(Ending::Told),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

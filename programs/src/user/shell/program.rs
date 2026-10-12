use crate::unit::UnitFile;
pub static PROGRAM: UnitFile = UnitFile {
    name: "shell",
    loader: crate::unit::LoaderConfig::user("shell"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        after: None,
        ready: crate::unit::Ready::OnSpawn,
        ending: None,

        construction: &[crate::unit::Construction::image()],
    },
    supply: crate::unit::SupplyConfig {
        bindings: crate::user::shell::commands::BINDINGS,
    },
};

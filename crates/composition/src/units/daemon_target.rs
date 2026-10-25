use crate::unit::{Ending, UnitFile};

/// Dedicated Root harness entry for the ordinary target restart scene.
pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon-target",
    loader: crate::unit::LoaderConfig::supervisor("daemon-target"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        ending: Some(Ending::Resident),
        ..crate::unit::ControlConfig::DEFAULT
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

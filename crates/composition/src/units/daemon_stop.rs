use crate::unit::{Ending, UnitFile};

/// Dedicated Root harness entry for the explicit-stop scene.
pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon-stop",
    loader: crate::unit::LoaderConfig::supervisor("daemon-stop"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        ending: Some(Ending::Resident),
        ..crate::unit::ControlConfig::DEFAULT
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

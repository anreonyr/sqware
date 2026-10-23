use crate::unit::{Ending, UnitFile};

/// Dedicated Root harness entry for the actual RTC restart scene.
pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon-rtc",
    loader: crate::unit::LoaderConfig::supervisor("daemon-rtc"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        ending: Some(Ending::Resident),
        ..crate::unit::ControlConfig::DEFAULT
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

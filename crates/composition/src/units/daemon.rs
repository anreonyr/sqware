//! Daemon UnitFile.

use crate::unit::{Ending, UnitFile};

pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon",
    loader: crate::unit::LoaderConfig::user("daemon"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        supervision: Some("supervised"),
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Target("supervised"),
        ]),
        ending: Some(Ending::Resident),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

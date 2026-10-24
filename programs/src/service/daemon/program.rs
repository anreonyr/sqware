//! Daemon UnitFile and its own exit reason.

use crate::unit::{Died, Ending, UnitFile};

/// The daemon could not open its scoped faces or lost that scope.
pub const E_DAEMON: Died = 13;

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

use crate::unit::{Dependency, Ending, UnitFile};

/// Transient controller that proves public scope denial, requests one stop and
/// waits for Root's ACK after the real merged Ruin.
pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon-ctl-stop",
    loader: crate::unit::LoaderConfig::user("daemon-ctl-stop"),
    operator: crate::unit::OperatorConfig::DEFAULT,
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[
            Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
            Dependency::Unit("fixturetarget"),
            Dependency::Unit("daemon"),
        ]),
        ending: Some(Ending::Transient),
        ready: crate::unit::Ready::OnSpawn,
        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

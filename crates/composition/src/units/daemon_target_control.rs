use crate::unit::{Dependency, Ending, UnitFile};

/// Transient controller that holds the runtime until the target restart proof.
pub static PROGRAM: UnitFile = UnitFile {
    name: "daemon-ctl-target",
    loader: crate::unit::LoaderConfig::user("daemon-ctl-target"),
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

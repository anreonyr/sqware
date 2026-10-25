use crate::unit::{Ending, UnitFile};

pub static ENTRY: UnitFile = UnitFile {
    name: "system-fault",
    loader: crate::unit::LoaderConfig::supervisor("system-fault"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: None,
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

pub static DEPENDENT: UnitFile = UnitFile {
    name: "system-dependent",
    loader: crate::unit::LoaderConfig::user("system-dependent"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Identity,
        )]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

pub static CHILD: UnitFile = UnitFile {
    name: "system-child",
    loader: crate::unit::LoaderConfig::user("system-child"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Identity,
        )]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

pub static FAULT_UNIT: UnitFile = UnitFile {
    name: "system-fault-unit",
    loader: crate::unit::LoaderConfig::supervisor("system-fault-unit"),
    operator: crate::unit::OperatorConfig { namespace: &[] },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: None,
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: None,

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

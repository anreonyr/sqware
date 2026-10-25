use crate::unit::{Ending, UnitFile};

/// Ordinary supervised target that owns one actual helper task in its Team.
pub static PROGRAM: UnitFile = UnitFile {
    name: "fixturetarget",
    loader: crate::unit::LoaderConfig::user("fixturetarget"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "fixturetarget",
            scope: crate::unit::operator::Scope::Subtree,
            ops: system_api::operator::Ops::PART
                .union(system_api::operator::Ops::UNPART)
                .union(system_api::operator::Ops::BIND)
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: Some(system_api::operator::Permit::Public),
            source_mark: Some(fixturetarget_api::ENTRY_MARK.get()),
            bindings: &[crate::unit::operator::Binding {
                name: fixturetarget_api::ENTRY_NAME,
                mark: fixturetarget_api::ENTRY_MARK,
                view: env::Access::STORE,
                permit: system_api::operator::Permit::Public,
            }],
        }],
    },
    identity: crate::unit::Identity::DEFAULT,
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[crate::unit::Dependency::Bootstrap(
            crate::unit::Bootstrap::Operator,
        )]),
        ending: Some(Ending::Resident),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig::DEFAULT,
};

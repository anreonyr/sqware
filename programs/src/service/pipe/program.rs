use crate::unit::{Ending, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    name: "pipe",
    loader: crate::unit::LoaderConfig::user("pipe"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            key: system_api::namespace::SERVICE,
            placement: crate::unit::operator::Placement::Shared,
            path: "pipe",
            scope: crate::unit::operator::Scope::Subtree,
            ops: system_api::operator::Ops::PART
                .union(system_api::operator::Ops::UNPART)
                .union(system_api::operator::Ops::BIND)
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: Some(system_api::operator::Permit::Public),
            source_mark: None,
            bindings: &[crate::unit::operator::Binding {
                name: "create",
                mark: pipe_api::ENTRY,
                view: env::Access::STORE,
                permit: system_api::operator::Permit::Public,
            }],
        }],
    },
    identity: crate::unit::Identity {
        aliases: &[],
        grants: &[],
    },
    control: crate::unit::ControlConfig {
        supervision: None,
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
        retirement: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

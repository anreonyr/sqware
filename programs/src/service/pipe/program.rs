use crate::unit::{Ending, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    name: "pipe",
    loader: crate::unit::LoaderConfig::user("pipe"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            path: "pipe",
            scope: crate::unit::operator::Scope::Names(&["create"]),
            ops: system_api::operator::Ops::BIND
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
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

use crate::unit::{Ending, UnitFile};
pub static PROGRAM: UnitFile = UnitFile {
    name: "account",
    loader: crate::unit::LoaderConfig::user("account"),
    operator: crate::unit::OperatorConfig {
        namespace: &[crate::unit::operator::Namespace {
            path: "account",
            scope: crate::unit::operator::Scope::Names(&["create"]),
            ops: system_api::operator::Ops::BIND
                .union(system_api::operator::Ops::UNBIND)
                .union(system_api::operator::Ops::FIND),
            permit: Some(system_api::operator::Permit::Bound),
            source_mark: None,
            bindings: &[crate::unit::operator::Binding {
                name: "create",
                mark: account_api::ENTRY,
                view: env::Access::STORE,
                permit: system_api::operator::Permit::Bound,
            }],
        }],
    },
    identity: crate::unit::Identity {
        aliases: &["anran"],
        grants: &[system_api::identity::Grant::Derive],
    },
    control: crate::unit::ControlConfig {
        after: Some(&[
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Operator),
            crate::unit::Dependency::Bootstrap(crate::unit::Bootstrap::Identity),
        ]),
        ready: crate::unit::Ready::Signal(crate::unit::READY_MARK),
        ending: Some(Ending::Resident),

        construction: &[crate::unit::Construction::unit_owned_by("shell", "login")],
    },
    supply: crate::unit::SupplyConfig { bindings: &[] },
};

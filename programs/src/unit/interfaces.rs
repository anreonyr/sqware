//! Interface registries selected by this program assembly.

const LOADER: &[&[env::marks::Definition]] = &[system_api::loader::REGISTRY];
pub const APIS: &[&[&[env::marks::Definition]]] = &[
    LOADER,
    account_api::REGISTRY,
    system_api::identity::REGISTRY,
    system_api::operator::REGISTRY,
    system_api::control::REGISTRY,
    hub_api::REGISTRY,
    terminal_api::REGISTRY,
    router_api::REGISTRY,
];

const _: () = assert!(env::marks::conflict_between(APIS).is_none(), "interface mark collision");

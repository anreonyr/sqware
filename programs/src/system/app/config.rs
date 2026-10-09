pub(crate) fn namespaces(machine: crate::support::machine::Machine) -> crate::system::publication::Namespaces {
    use crate::system::publication::{Namespace, Namespaces, Permission};
    use crate::unit::{Publish, PROGRAMS};
    use system_api::{control::publication::Scope, operator::Path};
    let mut rules = alloc::vec::Vec::new();
    for program in PROGRAMS {
        for declaration in program.publication {
            let (scope, group, road, entries, public) = match declaration {
                Publish::Entries { scope, group, road, entries, public } => (*scope, *group, *road, Some(entries.iter().map(|e| e.name.into()).collect()), *public),
                Publish::Namespace { scope, group, road, public } => (*scope, *group, *road, None, *public),
                Publish::Devices => {
                    if program.name() == hub_api::NAME {
                        let mut groups: alloc::collections::BTreeMap<alloc::string::String, alloc::vec::Vec<alloc::string::String>> = alloc::collections::BTreeMap::new();
                        groups.insert(hub_api::BOOT.into(), alloc::vec![hub_api::DTB.into(), hub_api::SUPERVISOR_EXTERNAL.into()]);
                        if let Some(devices) = machine.devices() {
                            for device in devices { groups.entry(device.class).or_default().push(device.name); }
                        }
                        for (group, entries) in groups {
                            rules.push(Namespace { owner: program.name(), scope: Scope(3), road: Path::new("dev").try_join(&group).expect("trusted device group"), group, entries: Some(entries), permission: Permission::Member, alias: true });
                        }
                    }
                    continue;
                }
            };
            rules.push(Namespace { owner: program.name(), scope: Scope(scope.0), group: group.into(), road: Path::new(road).to_path_buf(), entries, permission: if public { Permission::Public } else { Permission::Any }, alias: false });
        }
    }
    Namespaces(rules)
}

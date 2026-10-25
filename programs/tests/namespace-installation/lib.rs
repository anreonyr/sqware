#![allow(dead_code)]
extern crate alloc;
extern crate self as system_client;

pub mod debug {
    pub fn put(_: &str) {}
}

pub mod operator {
    #[derive(Clone, Copy)]
    pub struct GrantInfo {
        pub id: system_api::operator::GrantId,
        pub entry: system_api::operator::Entry,
        pub entrance: env::PieToken,
    }
}

pub use unit_declarations::operator as unit_operator;

pub mod unit {
    pub use crate::unit_operator as operator;

    pub struct Operator {
        pub namespace: &'static [operator::Namespace],
    }

    pub struct UnitFile {
        pub name: &'static str,
        pub operator: Operator,
    }

    impl UnitFile {
        pub fn name(&self) -> &'static str {
            self.name
        }
    }
}

pub mod system {
    pub mod app {
        pub mod namespace {
            use alloc::vec::Vec;
            use core::cell::RefCell;
            use env::{TaskId, Wait};
            use system_api::operator::{self as api, Entry, NamespaceKey, PathBuf};

            pub struct PrepareInput {
                pub base: Entry,
                pub prefix: PathBuf,
                pub retain_prefix: usize,
                pub declaration: crate::unit::operator::Namespace,
                pub grant_id: api::GrantId,
                pub holder: TaskId,
                pub source: Option<TaskId>,
                pub bindings: Vec<BindingInput>,
                pub wait: Wait,
            }

            pub struct ResourceInput {
                pub base: Entry,
                pub path: PathBuf,
                pub retain_prefix: usize,
                pub bindings: Vec<BindingInput>,
                pub wait: Wait,
            }

            pub struct BindingInput;

            pub struct Prepare {
                pub base: Entry,
                pub path: PathBuf,
                pub retain_prefix: usize,
                pub declaration: Option<crate::unit::operator::Namespace>,
            }

            impl Prepare {
                pub fn declared(input: PrepareInput) -> Option<Self> {
                    let path = if input.declaration.path.is_empty() {
                        input.prefix
                    } else {
                        input.prefix.as_path().try_join(input.declaration.path)?
                    };
                    Some(Self {
                        base: input.base,
                        path,
                        retain_prefix: input.retain_prefix,
                        declaration: Some(input.declaration),
                    })
                }

                pub fn resources(input: ResourceInput) -> Option<Self> {
                    Some(Self {
                        base: input.base,
                        path: input.path,
                        retain_prefix: input.retain_prefix,
                        declaration: None,
                    })
                }
            }

            pub struct Installed {
                pub anchor: Entry,
                pub authority: Option<crate::operator::GrantInfo>,
            }

            pub struct Preparation {
                pub plan: Prepare,
                pub installed: Installed,
            }

            impl Preparation {
                pub fn new(plan: Prepare) -> Self {
                    Self {
                        installed: Installed {
                            anchor: plan.base,
                            authority: None,
                        },
                        plan,
                    }
                }

                pub fn into_installed(self) -> Option<Installed> {
                    Some(self.installed)
                }
            }

            #[derive(Clone)]
            pub struct PreparedPath {
                pub base: Entry,
                pub path: String,
                pub retain_prefix: usize,
                pub declaration: Option<crate::unit::operator::Namespace>,
            }

            pub struct System {
                root: Entry,
                service: Entry,
                preparations: RefCell<Vec<PreparedPath>>,
                resolutions: RefCell<Vec<NamespaceKey>>,
                retirements: RefCell<Vec<usize>>,
                next_entry: core::cell::Cell<usize>,
            }

            impl Default for System {
                fn default() -> Self {
                    Self {
                        root: entry(0),
                        service: entry(1),
                        preparations: RefCell::new(Vec::new()),
                        resolutions: RefCell::new(Vec::new()),
                        retirements: RefCell::new(Vec::new()),
                        next_entry: core::cell::Cell::new(2),
                    }
                }
            }

            impl System {
                pub fn resolve(&self, key: NamespaceKey, _: Wait) -> Result<Entry, api::Fail> {
                    self.resolutions.borrow_mut().push(key);
                    if key == system_api::namespace::SERVICE {
                        Ok(self.service)
                    } else {
                        Err(api::Fail::Unknown)
                    }
                }

                pub fn prepare(&self, preparation: &mut Preparation) -> Result<(), api::Fail> {
                    let plan = &preparation.plan;
                    let id = self.next_entry.get();
                    self.next_entry.set(id + 1);
                    self.preparations.borrow_mut().push(PreparedPath {
                        base: plan.base,
                        path: plan.path.as_str().into(),
                        retain_prefix: plan.retain_prefix,
                        declaration: plan.declaration,
                    });
                    preparation.installed.anchor = entry(id);
                    Ok(())
                }

                pub fn retire(&self, installed: &mut Installed) -> Result<bool, api::Fail> {
                    self.retirements
                        .borrow_mut()
                        .push(entry_index(installed.anchor));
                    Ok(true)
                }

                pub fn retire_preparation(
                    &self,
                    preparation: &mut Preparation,
                ) -> Result<bool, api::Fail> {
                    self.retire(&mut preparation.installed)
                }

                pub fn preparations(&self) -> Vec<PreparedPath> {
                    self.preparations.borrow().clone()
                }

                pub fn resolutions(&self) -> Vec<NamespaceKey> {
                    self.resolutions.borrow().clone()
                }

                pub fn retirements(&self) -> Vec<usize> {
                    self.retirements.borrow().clone()
                }

                pub fn root(&self) -> Entry {
                    self.root
                }

                pub fn service(&self) -> Entry {
                    self.service
                }
            }

            fn entry(index: usize) -> Entry {
                Entry {
                    operator: TaskId::new(1),
                    target: if index == 0 {
                        api::EntryTarget::Root
                    } else {
                        api::EntryTarget::Node(api::EntryId::new(index))
                    },
                }
            }

            fn entry_index(entry: Entry) -> usize {
                match entry.target {
                    api::EntryTarget::Root => 0,
                    api::EntryTarget::Node(id) => id.get(),
                }
            }
        }
    }
}

#[path = "../../src/system/assembly/namespace.rs"]
mod production_namespace;

#[cfg(test)]
mod tests {
    use super::production_namespace::{
        Coordinates, NamespaceInstallations, PrepareTask,
    };
    use super::{system::app::namespace::System, unit};
    use env::{TaskId, TeamId, Wait};
    use system_api::operator::{self as api, NamespaceKey};

    const SERVICE: NamespaceKey = system_api::namespace::SERVICE;
    const UNKNOWN: NamespaceKey = NamespaceKey::of("unmapped.namespace");

    static INSTANCE_DECLARATIONS: [unit::operator::Namespace; 2] = [
        unit::operator::Namespace {
            key: SERVICE,
            placement: unit::operator::Placement::Instance,
            path: "private",
            scope: unit::operator::Scope::Subtree,
            ops: api::Ops::BIND.union(api::Ops::FIND),
            permit: None,
            source_mark: None,
            bindings: &[],
        },
        unit::operator::Namespace {
            key: SERVICE,
            placement: unit::operator::Placement::Instance,
            path: "control",
            scope: unit::operator::Scope::Names(&["state"]),
            ops: api::Ops::BIND.union(api::Ops::FIND),
            permit: None,
            source_mark: None,
            bindings: &[],
        },
    ];
    static INSTANCE_PROGRAM: unit::UnitFile = unit::UnitFile {
        name: "instance-example",
        operator: unit::Operator {
            namespace: &INSTANCE_DECLARATIONS,
        },
    };
    static UNKNOWN_DECLARATION: [unit::operator::Namespace; 1] = [
        unit::operator::Namespace {
            key: UNKNOWN,
            placement: unit::operator::Placement::Shared,
            path: "should-not-fallback",
            scope: unit::operator::Scope::Subtree,
            ops: api::Ops::BIND,
            permit: None,
            source_mark: None,
            bindings: &[],
        },
    ];
    static UNKNOWN_PROGRAM: unit::UnitFile = unit::UnitFile {
        name: "unknown-example",
        operator: unit::Operator {
            namespace: &UNKNOWN_DECLARATION,
        },
    };
    static EMPTY_PROGRAM: unit::UnitFile = unit::UnitFile {
        name: "empty-example",
        operator: unit::Operator { namespace: &[] },
    };

    fn coordinates(task: usize, team: Option<usize>) -> Coordinates {
        Coordinates {
            task: TaskId::new(task),
            team: team.map(TeamId::new),
        }
    }

    fn prepare(
        installations: &mut NamespaceInstallations,
        system: &System,
        task: usize,
        program: Option<&'static unit::UnitFile>,
    ) -> Result<bool, api::Fail> {
        installations.prepare_task(
            system,
            PrepareTask {
                task: TaskId::new(task),
                program,
                wait: Wait::POLL,
            },
        )
    }

    #[test]
    fn same_team_instance_tasks_get_distinct_roots_and_reuse_one_per_key() {
        let mut installations = NamespaceInstallations::new();
        let system = System::default();
        let a = coordinates(11, Some(7));
        let b = coordinates(12, Some(7));
        installations.coordinates(a).unwrap();
        installations.coordinates(b).unwrap();

        assert!(prepare(&mut installations, &system, 11, Some(&INSTANCE_PROGRAM)).unwrap());
        assert!(prepare(&mut installations, &system, 12, Some(&INSTANCE_PROGRAM)).unwrap());

        let paths = system.preparations();
        assert_eq!(paths.len(), 6, "one placement root plus two declarations per task");
        let placement_paths: Vec<_> = paths
            .iter()
            .filter(|path| path.declaration.is_none())
            .collect();
        assert_eq!(placement_paths.len(), 2, "two declarations with the same key reuse one root");
        assert_eq!(placement_paths[0].base, system.service());
        assert_eq!(placement_paths[1].base, system.service());
        assert_eq!(placement_paths[0].path, "7/11");
        assert_eq!(placement_paths[1].path, "7/12");
        assert_eq!(placement_paths[0].retain_prefix, 1);
        assert_eq!(placement_paths[1].retain_prefix, 1);

        let grants: Vec<_> = paths.iter().filter(|path| path.declaration.is_some()).collect();
        assert_eq!(grants.len(), 4);
        assert_eq!(grants[0].base, grants[1].base);
        assert_ne!(grants[0].base, grants[2].base);
        assert_eq!(grants[2].base, grants[3].base);
        assert_eq!(grants[0].path, "private");
        assert_eq!(grants[1].path, "control");
        assert_eq!(grants[2].path, "private");
        assert_eq!(grants[3].path, "control");
        assert!(installations.prepared(TaskId::new(11)));
        assert!(installations.prepared(TaskId::new(12)));
    }

    #[test]
    fn instance_without_team_is_rejected_before_any_system_side_effect() {
        let mut installations = NamespaceInstallations::new();
        installations.coordinates(coordinates(11, None)).unwrap();
        let system = System::default();

        assert_eq!(
            prepare(&mut installations, &system, 11, Some(&INSTANCE_PROGRAM)),
            Err(api::Fail::Denied)
        );
        assert!(system.preparations().is_empty());
        assert!(system.resolutions().is_empty());
        assert!(system.retirements().is_empty());
        assert!(!installations.prepared(TaskId::new(11)));
    }

    #[test]
    fn unknown_logical_key_does_not_fall_back_to_root_or_another_entry() {
        let mut installations = NamespaceInstallations::new();
        installations.coordinates(coordinates(20, Some(9))).unwrap();
        let system = System::default();

        assert_eq!(
            prepare(&mut installations, &system, 20, Some(&UNKNOWN_PROGRAM)),
            Err(api::Fail::Unknown)
        );
        assert_eq!(system.resolutions(), vec![UNKNOWN]);
        assert!(system.preparations().is_empty());
        assert!(!installations.prepared(TaskId::new(20)));
    }

    #[test]
    fn coordinates_are_immutable_and_failed_empty_receipt_does_not_block_another_task() {
        let mut installations = NamespaceInstallations::new();
        let a = coordinates(11, Some(7));
        installations.coordinates(a).unwrap();
        assert_eq!(
            installations.coordinates(coordinates(11, Some(9))),
            Err(api::Fail::Denied)
        );

        let system = System::default();
        assert_eq!(
            prepare(&mut installations, &system, 11, Some(&UNKNOWN_PROGRAM)),
            Err(api::Fail::Unknown)
        );
        assert!(installations.retire_task(&system, a.task).unwrap());

        installations.coordinates(coordinates(12, Some(8))).unwrap();
        assert!(prepare(&mut installations, &system, 12, Some(&EMPTY_PROGRAM)).unwrap());
        assert!(installations.prepared(TaskId::new(12)));
        assert!(installations.prepared_task(TaskId::new(12), Some(&EMPTY_PROGRAM)));
        assert!(system.preparations().is_empty());
    }
}

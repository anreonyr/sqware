#![allow(dead_code)]
extern crate alloc;
extern crate self as system_client;

pub mod operator {
    #[derive(Clone, Copy)]
    pub struct GrantInfo {
        pub id: system_api::operator::GrantId,
        pub entry: system_api::operator::Entry,
        pub entrance: env::PieToken,
    }
}

pub mod unit {
    pub mod operator {
        #[derive(Clone, Copy)]
        pub struct Namespace;
    }

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
            use core::cell::Cell;
            use env::{TaskId, Wait};
            use system_api::operator::{self as api, Entry, GrantId, PathBuf};
            use crate::operator::GrantInfo;

            #[derive(Clone, Copy)]
            pub struct Shared {
                pub root: Entry,
                pub services: Entry,
                pub drivers: Entry,
            }

            pub struct Preparation;
            pub struct Installed {
                pub authority: Option<GrantInfo>,
            }

            pub struct PrepareInput {
                pub base: Entry,
                pub prefix: PathBuf,
                pub declaration: crate::unit::operator::Namespace,
                pub grant_id: GrantId,
                pub holder: TaskId,
                pub source: Option<TaskId>,
                pub bindings: Vec<BindingInput>,
                pub wait: Wait,
            }

            pub struct BindingInput;
            pub struct Prepare;

            impl Prepare {
                pub fn declared(_: PrepareInput) -> Option<Self> {
                    Some(Self)
                }
            }

            impl Preparation {
                pub fn new(_: Prepare) -> Self {
                    Self
                }
                pub fn into_installed(self) -> Option<Installed> {
                    Some(Installed { authority: None })
                }
            }

            pub struct System {
                pub effects: Cell<usize>,
            }

            impl System {
                pub fn shared(&self) -> Shared {
                    let _ = api::OperationId(0);
                    let entry = Entry {
                        operator: TaskId::new(1),
                        target: api::EntryTarget::Root,
                    };
                    Shared {
                        root: entry,
                        services: entry,
                        drivers: entry,
                    }
                }

                pub fn prepare(&self, _: &mut Preparation) -> Result<(), api::Fail> {
                    self.effects.set(self.effects.get() + 1);
                    Ok(())
                }
                pub fn retire_preparation(&self, _: &mut Preparation) -> Result<bool, api::Fail> {
                    self.effects.set(self.effects.get() + 1);
                    Ok(true)
                }
                pub fn retire(&self, _: &mut Installed) -> Result<bool, api::Fail> {
                    self.effects.set(self.effects.get() + 1);
                    Ok(true)
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
        Coordinates, NamespaceInstallations, PrepareTask, team_task_prefix,
    };
    use super::{system::app::namespace::System, unit};
    use env::{TaskId, TeamId, Wait};

    static DECLARATIONS: [unit::operator::Namespace; 1] = [unit::operator::Namespace];
    static DECLARED: unit::UnitFile = unit::UnitFile {
        name: "unrouted",
        operator: unit::Operator {
            namespace: &DECLARATIONS,
        },
    };

    fn coordinates(task: usize, team: Option<usize>) -> Coordinates {
        Coordinates {
            task: TaskId::new(task),
            team: team.map(TeamId::new),
        }
    }

    #[test]
    fn failed_unrouted_receipt_can_be_retired_without_blocking_another_task() {
        let mut installations = NamespaceInstallations::new(&[]).unwrap();
        let a = coordinates(11, Some(7));
        let b = coordinates(12, Some(8));
        installations.coordinates(a).unwrap();
        assert_eq!(
            installations.coordinates(coordinates(11, Some(9))),
            Err(system_api::operator::Fail::Denied)
        );

        let system = System {
            effects: core::cell::Cell::new(0),
        };
        assert_eq!(
            installations.prepare_task(
                &system,
                PrepareTask {
                    task: a.task,
                    program: Some(&DECLARED),
                    wait: Wait::POLL,
                },
            ),
            Err(system_api::operator::Fail::Unknown)
        );
        assert_eq!(system.effects.get(), 0);

        installations.coordinates(b).unwrap();
        assert!(installations
            .prepare_task(
                &system,
                PrepareTask {
                    task: b.task,
                    program: None,
                    wait: Wait::POLL,
                },
            )
            .unwrap());
        assert!(installations.retire_task(&system, a.task).unwrap());
        assert!(installations.prepared(b.task));
        assert_eq!(system.effects.get(), 0);
    }

    #[test]
    fn team_prefix_uses_coordinates_and_requires_a_real_team() {
        assert_eq!(
            team_task_prefix(coordinates(12, Some(8))).unwrap().as_str(),
            "uit/8/12"
        );
        assert!(team_task_prefix(coordinates(12, None)).is_none());
    }
}

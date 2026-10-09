#![allow(dead_code)]
extern crate alloc;
extern crate self as resource;
use env::{PieToken, TaskId};
use system_api::control::publication::{self as api, Object, Target};
use system_api::identity::{PrincipalId, Subject};
use system_api::operator::{Fail, Permit};
pub mod raw {
    pub fn inspect(_: env::PieToken) -> Result<(env::TaskId, env::TaskId, env::Mark), ()> {
        panic!("identity alias must not inspect or consume a supplied capability")
    }
}
pub mod system {
    pub mod control {
        pub mod identity {
            use std::sync::Mutex;
            pub struct Roster {
                pub permitted: bool,
                pub calls: Mutex<Vec<(env::TaskId, system_api::identity::Subject)>>,
            }
            impl Roster {
                pub fn allow_subject(
                    &self,
                    from: env::TaskId,
                    subject: system_api::identity::Subject,
                ) -> Result<(), ()> {
                    self.calls.lock().unwrap().push((from, subject));
                    if self.permitted { Ok(()) } else { Err(()) }
                }
            }
            pub fn validate_permit(
                _: &Roster,
                _: system_api::operator::Permit,
            ) -> Result<(), system_api::operator::Fail> {
                Ok(())
            }
            pub fn binding(
                _: &Roster,
                _: env::TaskId,
            ) -> Result<Option<system_api::identity::Binding>, system_api::operator::Fail>
            {
                unreachable!()
            }
        }
        pub mod unit {
            pub struct Identity {
                pub aliases: &'static [&'static str],
            }
            pub struct Program {
                pub identity: Identity,
            }
            pub struct Input {
                pub program: Program,
            }
            pub struct Row {
                pub name: alloc::string::String,
            }
            pub struct Control {
                pub live: bool,
                pub named: bool,
                pub input: Input,
            }
            impl Control {
                pub fn live(&self, _: env::TaskId) -> bool {
                    self.live
                }
                pub fn find_named_task(&self, _: env::TaskId) -> Option<Row> {
                    self.named.then(|| Row {
                        name: "provider".into(),
                    })
                }
                pub fn input(&self, _: &str) -> Result<&Input, ()> {
                    Ok(&self.input)
                }
            }
        }
    }
    pub mod operator {
        pub struct Placement {
            pub road: system_api::operator::PathBuf,
            pub tile: tree::Tile,
            pub replace: bool,
        }
        pub mod tree {
            pub struct Tile {
                pub pie: env::PieToken,
                pub permit: system_api::operator::Permit,
                pub owner: Option<env::TaskId>,
            }
        }
    }
    pub mod publication {
        pub mod runtime {
            pub struct Resources;
            impl Resources {
                pub(crate) fn policy(
                    &self,
                    _: &crate::Incoming,
                ) -> Result<crate::Decision, system_api::operator::Fail> {
                    Err(system_api::operator::Fail::Denied)
                }
            }
        }
    }
}
#[path = "../../src/system/publication/admission.rs"]
mod admission;
use admission::Namespaces;
struct Incoming {
    frame: api::Frame,
    from: TaskId,
    back: Option<()>,
}
struct Request(Option<Incoming>, Option<alloc::string::String>);
struct Registration {
    name: alloc::string::String,
    object: Object,
    lifetime: Option<TaskId>,
}
mod names {
    pub(crate) use super::Registration;
}
struct Approval {
    target: Target,
    member: bool,
    alias: bool,
}
struct Approved {
    policy: Approval,
    placement: system::operator::Placement,
    publisher: TaskId,
}
enum Decision {
    Unset,
    Failed(Fail),
    BindAlias {
        publisher: TaskId,
        registration: Registration,
    },
    OwnHole(Approved),
    Install(Approved),
}
#[path = "../../src/system/publication/policy.rs"]
mod policy;
fn decide(
    live: bool,
    named: bool,
    aliases: &'static [&'static str],
    permitted: bool,
    target: Target,
    seed: PieToken,
    permit: Permit,
) -> (Decision, Vec<(TaskId, Subject)>) {
    use system::control::{
        identity::Roster,
        unit::{Control, Identity, Input, Program},
    };
    let mut resources = schedule::Resources::new();
    resources
        .insert(Request(
            Some(Incoming {
                frame: api::Frame::new(api::PUBLISH, target, (seed, permit)),
                from: TaskId::new(7),
                back: Some(()),
            }),
            None,
        ))
        .unwrap();
    resources.insert(Decision::Unset).unwrap();
    resources
        .insert(Control {
            live,
            named,
            input: Input {
                program: Program {
                    identity: Identity { aliases },
                },
            },
        })
        .unwrap();
    resources
        .insert(Roster {
            permitted,
            calls: std::sync::Mutex::new(Vec::new()),
        })
        .unwrap();
    let mut plan = schedule::Schedule::sequence();
    plan.system("source", policy::source).unwrap();
    plan.system("alias", policy::alias).unwrap();
    plan.system("identity", policy::identity).unwrap();
    let mut plan = plan.build().unwrap();
    plan.prepare(&resources);
    assert_eq!(
        plan.advance(&mut schedule::Cursor::default(), &resources)
            .unwrap(),
        schedule::Progress::Done
    );
    let calls = resources
        .read::<Roster>()
        .unwrap()
        .calls
        .lock()
        .unwrap()
        .clone();
    let decision = core::mem::replace(
        &mut *resources.write::<Decision>().unwrap(),
        Decision::Unset,
    );
    (decision, calls)
}
fn principal() -> Object {
    Object::Principal(PrincipalId::new(TaskId::new(3), 19))
}
fn target() -> Target {
    Target::IdentityName {
        object: principal(),
        name: "account-name".into(),
    }
}
#[test]
fn declared_named_live_publisher_still_requires_identity_subtree_authorization() {
    let (decision, calls) = decide(
        true,
        true,
        &["account-name"],
        true,
        target(),
        PieToken::NONE,
        Permit::Bound,
    );
    assert!(
        matches!(decision, Decision::BindAlias { publisher, registration } if publisher == TaskId::new(7) && registration.object == principal() && registration.lifetime == Some(TaskId::new(7)))
    );
    assert_eq!(
        calls,
        [(
            TaskId::new(7),
            Subject::new(PrincipalId::new(TaskId::new(3), 19), &[]).unwrap()
        )]
    );
    let (decision, calls) = decide(
        true,
        true,
        &["account-name"],
        false,
        target(),
        PieToken::NONE,
        Permit::Bound,
    );
    assert!(matches!(decision, Decision::Failed(Fail::Denied)));
    assert_eq!(calls.len(), 1);
}
#[test]
fn aliases_are_not_available_to_unnamed_children_or_undeclared_names() {
    for (live, named, aliases) in [
        (false, true, &["account-name"][..]),
        (true, false, &["account-name"][..]),
        (true, true, &["other"][..]),
    ] {
        let (decision, calls) = decide(
            live,
            named,
            aliases,
            true,
            target(),
            PieToken::NONE,
            Permit::Bound,
        );
        assert!(matches!(decision, Decision::Failed(Fail::Denied)));
        assert!(calls.is_empty());
    }
}
#[test]
fn identity_alias_rejects_capability_payload_public_permit_and_coalition() {
    for (target, seed, permit) in [
        (target(), PieToken::mint(9), Permit::Bound),
        (target(), PieToken::NONE, Permit::Public),
        (
            Target::IdentityName {
                object: Object::Coalition(system_api::identity::CoalitionId::new(
                    TaskId::new(3),
                    19,
                )),
                name: "account-name".into(),
            },
            PieToken::NONE,
            Permit::Bound,
        ),
    ] {
        let (decision, calls) = decide(true, true, &["account-name"], true, target, seed, permit);
        assert!(matches!(decision, Decision::Failed(Fail::Denied)));
        assert!(calls.is_empty());
    }
}

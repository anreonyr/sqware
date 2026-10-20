#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as resource;

pub use abi::wire;
pub use abi::{Access, Frame, HoleLimits, HoleResult, Mark, Permission, PieFail, PieInfo, PieKind, PieResult, PieToken, Policy, ReleaseMode, TaskId, Wait};

#[path = "../../src/unit/construction.rs"]
mod unit_construction;

use std::cell::RefCell;

#[derive(Default)]
struct Effects {
    fail_ship_at: Option<usize>,
    fail_release: bool,
    unseals: Vec<PieToken>,
    ships: Vec<(PieToken, TaskId, Access, Policy)>,
    releases: Vec<PieToken>,
}

thread_local! { static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default()); }

pub mod hole {
    pub fn unseal(_: crate::Mark, _: crate::HoleLimits) -> crate::HoleResult<crate::PieToken> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let token = crate::PieToken::mint(effects.unseals.len() + 1);
            effects.unseals.push(token);
            Ok(token)
        })
    }
}

pub mod pie {
    pub fn release(token: crate::PieToken, _: crate::ReleaseMode) -> crate::PieResult<()> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.releases.push(token);
            if effects.fail_release {
                Err(erra::Error::new("injected release failure", crate::PieFail::Denied))
            } else { Ok(()) }
        })
    }
}

pub mod port {
    pub fn ship(
        entry: crate::PieToken,
        task: crate::TaskId,
        access: crate::Access,
        policy: crate::Policy,
    ) -> Result<(), ()> {
        crate::EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.ships.push((entry, task, access, policy));
            if effects.fail_ship_at == Some(effects.ships.len()) { Err(()) } else { Ok(()) }
        })
    }
}

pub mod raw {
    pub fn reserve(_: crate::PieToken) -> Result<(crate::TaskId, crate::TaskId, crate::Mark), ()> {
        Ok((crate::TaskId::new(1), crate::TaskId::new(1), system_api::control::construction::Call::BACK))
    }
    pub fn release(token: crate::PieToken) -> crate::PieResult<()> {
        crate::pie::release(token, crate::ReleaseMode::Revoke)
    }
    pub fn inspect(token: crate::PieToken) -> Result<crate::PieInfo, ()> {
        let constructor = crate::EFFECTS.with(|effects| effects.borrow().unseals.contains(&token));
        if constructor {
            return Ok(crate::PieInfo {
                token, kind: crate::PieKind::Hole,
                permission: crate::Permission::FETCH | crate::Permission::STORE | crate::Permission::VEST,
                owner: crate::TaskId::new(1), vestor: crate::TaskId::new(0),
                mark: system_api::control::construction::ENTRY, alive: true,
            });
        }
        let requester = crate::TaskId::new(7);
        Ok(crate::PieInfo {
            token,
            kind: crate::PieKind::Pole,
            permission: abi::Permission::FETCH | abi::Permission::VEST,
            owner: requester,
            vestor: requester,
            mark: system_api::loader::IMAGE,
            alive: true,
        })
    }
}

pub mod system {
    pub mod app { pub type Fault = &'static str; }
    pub mod control {
        pub mod unit {
            pub struct Control;
            impl Control {
                pub fn live(&self, _: crate::TaskId) -> bool { true }
                pub fn named_task(&self, _: &str) -> Result<crate::TaskId, ()> { Ok(crate::TaskId::new(9)) }
            }
        }
        pub mod identity {
            pub struct Roster;
            impl Roster {
                pub fn allow_subject(&self, task: crate::TaskId, subject: system_api::identity::Subject) -> Result<(), ()> {
                    if task == crate::TaskId::new(9) && subject == authorized_subject() { Ok(()) } else { Err(()) }
                }
            }
            pub fn binding(_: &Roster, task: crate::TaskId) -> Result<Option<system_api::identity::Binding>, ()> {
                Ok((task == crate::TaskId::new(7)).then_some(system_api::identity::Binding {
                    origin: authorized_subject(),
                    current: authorized_subject(),
                }))
            }
            fn authorized_subject() -> system_api::identity::Subject {
                system_api::identity::Subject {
                    principal: system_api::identity::PrincipalId::new(crate::TaskId::new(100), 1),
                    coalitions: system_api::identity::CoalitionSet::empty(),
                }
            }
        }
    }
}

#[path = "../../src/system/control/endpoint/construction.rs"]
mod construction;

pub mod unit {
    pub use crate::unit_construction::{Arguments, Construction, Identity, Owner, Range, Selector};
    pub type ConstructionIdentity = Identity;
    pub type ConstructionOwner = Owner;
    pub type ConstructionSelector = Selector;
    pub fn self_id() -> crate::TaskId { crate::TaskId::new(1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() { EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default()); }

    #[test]
    fn shell_unit_policy_is_narrow_and_image_defaults_bind_owner_and_identity_to_caller() {
        let shell = unit::Construction::unit("shell");
        assert!(matches!(shell.selector, unit::Selector::Unit("shell")));
        assert!(matches!(shell.owner, unit::Owner::Selected));
        assert!(matches!(shell.identity, unit::Identity::Selected));
        let image = unit::Construction::image();
        assert!(matches!(image.selector, unit::Selector::Image));
        assert!(matches!(image.owner, unit::Owner::Caller));
        assert!(matches!(image.identity, unit::Identity::Caller));
    }

    #[test]
    fn policy_entries_are_independent_by_holder_and_replay_grants_no_extra_store() {
        reset();
        static POLICIES: [unit::Construction; 2] = [unit::Construction::image(), unit::Construction::unit("shell")];
        let mut inlet = construction::Construction::new().unwrap();
        inlet.grant(TaskId::new(7), &POLICIES).unwrap();
        let first: alloc::vec::Vec<_> = inlet.entries().collect();
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1]);
        inlet.grant(TaskId::new(7), &POLICIES).unwrap();
        inlet.grant(TaskId::new(8), &POLICIES).unwrap();
        let all: alloc::vec::Vec<_> = inlet.entries().collect();
        assert_eq!(all.len(), 4);
        for index in 0..all.len() {
            assert!(!all[..index].contains(&all[index]));
        }
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(effects.ships.len(), 4);
            assert!(effects.ships.iter().all(|(_, _, access, policy)| *access == Access::STORE && *policy == Policy::NONE));
        });
    }

    #[test]
    fn failed_grant_is_not_registered_and_failed_retire_keeps_roots_retryable() {
        reset();
        static POLICIES: [unit::Construction; 2] = [unit::Construction::image(), unit::Construction::unit("shell")];
        let mut inlet = construction::Construction::new().unwrap();
        EFFECTS.with(|effects| effects.borrow_mut().fail_ship_at = Some(2));
        assert!(inlet.grant(TaskId::new(7), &POLICIES).is_err());
        assert_eq!(inlet.entries().count(), 1);
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(effects.ships.len(), 2);
            assert_eq!(effects.releases.len(), 1);
        });

        EFFECTS.with(|effects| effects.borrow_mut().fail_ship_at = None);
        inlet.grant(TaskId::new(7), &POLICIES).unwrap();
        let owned: alloc::vec::Vec<_> = inlet.entries().collect();
        EFFECTS.with(|effects| effects.borrow_mut().fail_release = true);
        assert!(inlet.retire(TaskId::new(7), |_, _| Ok(())).is_err());
        assert_eq!(inlet.entries().count(), 2);
        EFFECTS.with(|effects| effects.borrow_mut().fail_release = false);
        inlet.retire(TaskId::new(7), |_, _| Ok(())).unwrap();
        assert_eq!(inlet.entries().count(), 0);
        assert!(owned.iter().all(|entry| EFFECTS.with(|effects| effects.borrow().releases.contains(entry))));
    }

    #[test]
    fn replay_at_capacity_reuses_roots_and_new_holder_is_full_before_unseal() {
        reset();
        static POLICY: [unit::Construction; 1] = [unit::Construction::image()];
        let mut inlet = construction::Construction::new().unwrap();
        for holder in 1..=256 { inlet.grant(TaskId::new(holder), &POLICY).unwrap(); }
        inlet.grant(TaskId::new(1), &POLICY).unwrap();
        assert!(inlet.grant(TaskId::new(257), &POLICY).is_err());
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert_eq!(effects.unseals.len(), 256);
            assert_eq!(effects.ships.len(), 256);
        });
    }

    #[test]
    fn failed_delivery_and_failed_revoke_keep_the_same_root_for_retry() {
        reset();
        static POLICY: [unit::Construction; 1] = [unit::Construction::image()];
        let mut inlet = construction::Construction::new().unwrap();
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_ship_at = Some(1); effects.fail_release = true;
        });
        assert!(inlet.grant(TaskId::new(7), &POLICY).is_err());
        let pending: Vec<_> = inlet.entries().collect();
        assert_eq!(pending.len(), 1);
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            effects.fail_ship_at = None; effects.fail_release = false;
        });
        inlet.grant(TaskId::new(7), &POLICY).unwrap();
        assert_eq!(inlet.entries().collect::<Vec<_>>(), pending);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().unseals.len(), 1));
        assert!(inlet.retire(TaskId::new(7), |_, _| Err(())).is_err());
        assert_eq!(inlet.entries().count(), 1);
        inlet.retire(TaskId::new(7), |_, _| Ok(())).unwrap();
        assert_eq!(inlet.entries().count(), 0);
        EFFECTS.with(|effects| assert_eq!(effects.borrow().releases.len(), 2));
    }

    #[test]
    fn invalid_policy_is_rejected_before_any_native_effect() {
        reset();
        static INVALID: [unit::Construction; 1] = [unit::Construction {
            selector: unit::Selector::Unit(""), owner: unit::Owner::Caller,
            identity: unit::Identity::Caller,
            arguments: unit::Arguments { min: 0, max: 1 },
            stack: unit::Range { min: 0, max: 1 }, max_image: 0,
        }];
        let mut inlet = construction::Construction::new().unwrap();
        assert!(inlet.grant(TaskId::new(7), &INVALID).is_err());
        assert_eq!(inlet.entries().count(), 0);
        EFFECTS.with(|effects| {
            let effects = effects.borrow();
            assert!(effects.unseals.is_empty());
            assert!(effects.ships.is_empty());
        });
    }
}

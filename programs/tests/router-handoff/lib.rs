#![allow(dead_code)]

extern crate self as ipc;
extern crate self as resource;

use env::{Mark, PieToken, TaskId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

#[derive(Clone, Copy)]
struct Facts {
    alive: bool,
    vestor: TaskId,
    owner: TaskId,
    mark: Mark,
}

static FACTS: OnceLock<Mutex<HashMap<usize, Facts>>> = OnceLock::new();
static ACCEPTED: AtomicUsize = AtomicUsize::new(usize::MAX);
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn facts() -> &'static Mutex<HashMap<usize, Facts>> {
    FACTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reset() -> MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap();
    facts().lock().unwrap().clear();
    ACCEPTED.store(usize::MAX, Ordering::SeqCst);
    guard
}

fn token(value: u64) -> PieToken {
    PieToken::from_bytes(&value.to_le_bytes()).unwrap()
}

fn register(token: PieToken, alive: bool, vestor: TaskId, owner: TaskId, mark: Mark) {
    facts().lock().unwrap().insert(
        token.get(),
        Facts {
            alive,
            vestor,
            owner,
            mark,
        },
    );
}

pub mod raw {
    use super::{Facts, facts};
    use env::PieToken;

    pub fn alive(token: PieToken) -> bool {
        facts()
            .lock()
            .unwrap()
            .get(&token.get())
            .is_some_and(|fact| fact.alive)
    }

    pub fn reserve(token: PieToken) -> Result<(env::TaskId, env::TaskId, env::Mark), ()> {
        facts()
            .lock()
            .unwrap()
            .get(&token.get())
            .copied()
            .map(
                |Facts {
                     vestor,
                     owner,
                     mark,
                     ..
                 }| (vestor, owner, mark),
            )
            .ok_or(())
    }
}

pub mod session {
    use super::{ACCEPTED, Ordering};
    use env::PieToken;

    pub struct Endpoint {
        tx: PieToken,
        seed: PieToken,
    }
    impl Endpoint {
        pub fn tx(&self) -> Option<PieToken> {
            Some(self.tx)
        }
        pub fn seed(&self) -> PieToken {
            self.seed
        }
    }

    pub struct Held(pub Endpoint);
    impl core::ops::Deref for Held {
        type Target = Endpoint;
        fn deref(&self) -> &Endpoint {
            &self.0
        }
    }

    pub mod establish {
        use super::{ACCEPTED, Endpoint, Ordering};
        use env::PieToken;

        pub fn accept(token: PieToken) -> Result<super::Endpoint, ()> {
            ACCEPTED.store(token.get(), Ordering::SeqCst);
            Ok(Endpoint {
                tx: token,
                seed: PieToken::mint(token.get() + 1000),
            })
        }
    }
}

#[path = "../../src/driver/router/client/src/handoff.rs"]
mod client_handoff;
#[path = "../../src/driver/router/adapt/event/handoff.rs"]
mod handoff;

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> TaskId {
        TaskId::new(41)
    }

    #[test]
    fn lane_uses_the_supplied_seed_even_with_another_matching_lane_present() {
        let _guard = reset();
        let chosen = token(11);
        register(
            chosen,
            true,
            owner(),
            owner(),
            Mark::of(router_api::frame::LANE),
        );
        register(
            token(12),
            true,
            owner(),
            owner(),
            Mark::of(router_api::frame::LANE),
        );

        let lane = handoff::lane(owner(), chosen).unwrap();

        assert_eq!(ACCEPTED.load(Ordering::SeqCst), chosen.get());
        assert_eq!(lane.tx(), Some(chosen));
        assert_eq!(lane.seed(), PieToken::mint(chosen.get() + 1000));
    }

    #[test]
    fn lane_rejects_dead_wrong_owner_wrong_vestor_and_wrong_role_before_accept() {
        let _guard = reset();
        let wrong_vestor = token(21);
        register(
            wrong_vestor,
            true,
            TaskId::new(9),
            owner(),
            Mark::of(router_api::frame::LANE),
        );
        assert!(handoff::lane(owner(), wrong_vestor).is_none());

        let wrong_owner = token(22);
        register(
            wrong_owner,
            true,
            owner(),
            TaskId::new(9),
            Mark::of(router_api::frame::LANE),
        );
        assert!(handoff::lane(owner(), wrong_owner).is_none());

        let wrong_role = token(23);
        register(wrong_role, true, owner(), owner(), Mark::of("wrong-role"));
        assert!(handoff::lane(owner(), wrong_role).is_none());

        let dead = token(24);
        register(
            dead,
            false,
            owner(),
            owner(),
            Mark::of(router_api::frame::LANE),
        );
        assert!(handoff::lane(owner(), dead).is_none());
        assert_eq!(ACCEPTED.load(Ordering::SeqCst), usize::MAX);
    }

    #[test]
    fn reply_accepts_only_the_explicit_live_back_capability() {
        let _guard = reset();
        let back = token(31);
        register(back, true, owner(), owner(), router_api::frame::BACK_MARK);
        assert_eq!(handoff::back(owner(), back), Some(back));

        let wrong = token(32);
        register(wrong, true, owner(), owner(), Mark::of("other"));
        assert_eq!(handoff::back(owner(), wrong), None);
        let foreign = token(33);
        register(
            foreign,
            true,
            owner(),
            TaskId::new(7),
            router_api::frame::BACK_MARK,
        );
        assert_eq!(handoff::back(owner(), foreign), None);
    }

    #[test]
    fn client_import_requires_a_live_lane_from_the_expected_router() {
        let _guard = reset();
        let lane = token(41);
        register(lane, true, owner(), owner(), router_api::LINE_MARK);
        assert!(client_handoff::valid_lane(owner(), lane));

        let wrong_vestor = token(42);
        register(
            wrong_vestor,
            true,
            TaskId::new(7),
            owner(),
            router_api::LINE_MARK,
        );
        assert!(!client_handoff::valid_lane(owner(), wrong_vestor));
        let wrong_owner = token(43);
        register(
            wrong_owner,
            true,
            owner(),
            TaskId::new(7),
            router_api::LINE_MARK,
        );
        assert!(!client_handoff::valid_lane(owner(), wrong_owner));
        let wrong_mark = token(44);
        register(wrong_mark, true, owner(), owner(), Mark::of("wrong-role"));
        assert!(!client_handoff::valid_lane(owner(), wrong_mark));
        let dead = token(45);
        register(dead, false, owner(), owner(), router_api::LINE_MARK);
        assert!(!client_handoff::valid_lane(owner(), dead));
        assert!(!client_handoff::valid_lane(owner(), PieToken::NONE));
        assert!(client_handoff::valid_source(Some(owner()), owner()));
        assert!(!client_handoff::valid_source(Some(TaskId::new(7)), owner()));
        assert!(!client_handoff::valid_source(None, owner()));
    }
}

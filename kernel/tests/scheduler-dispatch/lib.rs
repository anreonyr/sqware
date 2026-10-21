#![cfg(test)]

#[path = "fixtures/state.rs"]
mod state;
#[path = "fixtures/guard.rs"]
mod guard;
#[path = "fixtures/conductor.rs"]
mod fixture;

mod core {
    pub mod work {
        pub fn commit() -> crate::guard::Guard {
            crate::state::STATE.with(|state| state.borrow_mut().depth += 1);
            crate::guard::Guard
        }

        pub mod room {
            pub mod scheduler {
                pub fn steal() -> Option<usize> {
                    crate::state::dequeue(true)
                }
            }
        }
    }
}

mod conductor {
    fn current() -> &'static crate::fixture::Conductor {
        &crate::fixture::Conductor
    }

    mod dispatch {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/core/work/room/conductor/dispatch.rs"
        ));
    }

    fn run(local: bool, remote: bool) {
        crate::state::STATE.with(|state| {
            *state.borrow_mut() = crate::state::State {
                local,
                remote,
                ..Default::default()
            };
        });
        assert_eq!(dispatch::dispatch(), Some(0x1000));
        crate::state::STATE.with(|state| {
            let state = state.borrow();
            assert_eq!(state.depth, 0);
            assert!(state.seated && state.slain);
            assert!(!state.ready_outside_queue);
        });
    }

    #[test]
    fn local_dequeue_and_seat_are_one_commit_against_slay() {
        run(true, false);
    }

    #[test]
    fn stolen_task_and_seat_are_one_commit_against_slay() {
        run(false, true);
    }

    #[test]
    fn empty_queues_release_commit_without_seating() {
        crate::state::STATE.with(|state| *state.borrow_mut() = Default::default());
        assert_eq!(dispatch::dispatch(), None);
        crate::state::STATE.with(|state| {
            let state = state.borrow();
            assert_eq!(state.depth, 0);
            assert!(!state.seated && !state.slain);
        });
    }
}

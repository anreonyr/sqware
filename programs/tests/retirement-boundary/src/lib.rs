#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate system_api as abi_system;

pub use abi::{PieToken, TaskId, TeamId, Wait};

#[path = "fixtures/chrono.rs"]
pub mod chrono;
#[path = "fixtures/programs.rs"]
pub mod programs;
#[path = "fixtures/unit/mod.rs"]
pub mod unit;
#[path = "fixtures/ipc/mod.rs"]
pub mod ipc;
#[path = "fixtures/system_api/mod.rs"]
pub mod system_api;
#[path = "fixtures/system_client/mod.rs"]
pub mod system_client;
#[path = "fixtures/backend/mod.rs"]
mod backend;
#[path = "fixtures/system/mod.rs"]
pub mod system;

#[cfg(test)]
mod tests {
    use super::*;
    use system::app::policy::{Flow, Settle};
    use unit::{ControlConfig, Retirement, UnitFile};

    static HUB: Retirement = Retirement {
        provider: "hub",
        namespace: 0,
        face: "retire",
    };
    static SECOND: Retirement = Retirement {
        provider: "provider",
        namespace: 1,
        face: "settle",
    };
    static DEVICE: UnitFile = UnitFile {
        name: "device",
        control: ControlConfig {
            retirement: &[HUB],
        },
    };
    static BOTH: UnitFile = UnitFile {
        name: "both",
        control: ControlConfig {
            retirement: &[HUB, SECOND],
        },
    };
    static EMPTY: UnitFile = UnitFile {
        name: "hub",
        control: ControlConfig { retirement: &[] },
    };
    static DEPENDENT: UnitFile = UnitFile {
        name: "worker",
        control: ControlConfig {
            retirement: &[HUB],
        },
    };

    fn flow() -> Flow {
        Flow {
            settling: false,
            forced: false,
            done: false,
        }
    }

    fn done() -> Result<Option<system_api::retirement::Retirement>, ()> {
        Ok(Some(system_api::retirement::Retirement::done()))
    }

    #[test]
    fn different_instance_replies_cannot_discharge_each_others_obligations() {
        use system::assembly::retirement::Participant;
        use crate::system_api::retirement::Retirement as Reply;
        let (a, b) = (TaskId::new(7), TaskId::new(8));
        backend::reset(vec![Ok(()), Ok(())], vec![Ok(()); 4], vec![
            Ok(Some(Reply::done_for(b))), Ok(Some(Reply::done_for(a))),
            Ok(Some(Reply::done_for(a))), Ok(Some(Reply::done_for(b))),
        ]);
        let mut first = Participant::new(HUB, a);
        let mut second = Participant::new(HUB, b);
        let (system, namespaces) = (system::app::namespace::System, system::assembly::namespace::NamespaceInstallations);
        first.discover(&system, &namespaces);
        second.discover(&system, &namespaces);
        first.advance();
        second.advance();
        assert!(!first.done && !second.done);
        first.advance();
        assert!(first.done && !second.done);
        second.advance();
        assert!(second.done);
        assert_eq!(backend::targets(), [a, b, a, b]);
    }

    #[test]
    fn instance_done_does_not_complete_global_retirement() {
        use crate::system_api::retirement::Retirement as Reply;
        backend::reset(vec![Ok(())], vec![Ok(()), Ok(())], vec![
            Ok(Some(Reply::done_for(TaskId::new(7)))), done(),
        ]);
        let mut gate = settle(&DEVICE);
        discover(&mut gate);
        assert!(!gate.admit(false));
        assert!(!gate.settled());
        assert!(gate.admit(false));
        assert_eq!(backend::targets(), [TaskId::new(0); 2]);
    }

    fn settle(program: &'static UnitFile) -> Settle {
        Settle::new(flow(), &[program])
    }

    fn discover(settle: &mut Settle) {
        settle.discover(
            Some(&system::app::namespace::System),
            &system::assembly::namespace::NamespaceInstallations,
        );
    }

    #[test]
    fn missing_grant_or_failed_discovery_holds_normal_and_forced_entry() {
        backend::reset(vec![], vec![], vec![]);
        backend::deny_grant();
        let mut missing = settle(&DEVICE);
        discover(&mut missing);
        assert!(!missing.settled());
        assert!(!missing.admit(false));
        assert!(!missing.admit(true));
        assert!(!missing.flow.settling);

        backend::reset(vec![Err(())], vec![], vec![]);
        let mut failed = settle(&DEVICE);
        discover(&mut failed);
        assert!(!failed.settled());
        assert!(!failed.admit(false));
        assert!(!failed.admit(true));
        assert!(!failed.flow.settling);
    }

    #[test]
    fn refused_begin_and_poll_timeout_do_not_settle() {
        backend::reset(vec![Ok(())], vec![Err(())], vec![]);
        let mut refused = settle(&DEVICE);
        discover(&mut refused);
        assert!(!refused.admit(false));
        assert_eq!(backend::calls(), (1, 1, 0));

        backend::reset(vec![Ok(())], vec![Ok(())], vec![Err(())]);
        let mut timeout = settle(&DEVICE);
        discover(&mut timeout);
        assert!(!timeout.admit(false));
        assert_eq!(backend::calls(), (1, 1, 1));
    }

    #[test]
    fn in_flight_pending_is_polled_without_rediscovery_then_completes() {
        backend::reset(
            vec![Ok(())],
            vec![Ok(())],
            vec![Ok(None), done()],
        );
        let mut settle = settle(&DEVICE);
        discover(&mut settle);
        assert!(!settle.admit(false));
        assert!(settle.working());
        assert_eq!(backend::calls(), (1, 1, 1));
        assert!(settle.admit(false));
        assert!(settle.flow.settling);
        assert_eq!(backend::calls(), (1, 1, 2));
    }

    #[test]
    fn one_pending_participant_blocks_the_global_gate() {
        backend::reset(
            vec![Ok(()), Ok(())],
            vec![Ok(()), Ok(())],
            vec![done(), Ok(None), done()],
        );
        let mut settle = settle(&BOTH);
        discover(&mut settle);
        assert!(!settle.admit(false));
        assert!(!settle.settled());
        assert!(settle.settled_for(&[HUB]));
        assert!(!settle.settled_for(&[HUB, SECOND]));
        assert_eq!(backend::calls(), (2, 2, 2));
        assert!(settle.admit(false));
        assert!(settle.flow.settling);
        assert_eq!(backend::calls(), (2, 2, 3));
    }

    #[test]
    fn static_gate_uses_declarations_not_program_names() {
        backend::reset(vec![], vec![], vec![]);
        let mut settle = settle(&DEPENDENT);
        assert!(settle.gate(&EMPTY));
        assert!(!settle.gate(&DEPENDENT));
        assert_eq!(backend::calls(), (0, 0, 0));
    }

    #[test]
    fn undeployed_programs_do_not_add_global_requirements() {
        backend::reset(vec![], vec![], vec![]);
        let mut settle = Settle::new(flow(), &[&EMPTY]);
        assert!(settle.settled());
        assert!(!settle.gate(&DEVICE));
        assert!(settle.admit(false));
        assert_eq!(backend::calls(), (0, 0, 0));
    }

    #[test]
    fn unknown_reply_keeps_the_declared_requirement_and_retries() {
        backend::reset(
            vec![Ok(())],
            vec![Ok(()), Ok(())],
            vec![Ok(Some(system_api::retirement::Retirement { state: 255, ..system_api::retirement::Retirement::pending() })), done()],
        );
        let mut settle = settle(&DEVICE);
        discover(&mut settle);
        assert!(!settle.admit(false));
        assert!(!settle.flow.settling);
        assert!(settle.admit(false));
        assert_eq!(backend::calls(), (1, 2, 2));
    }

    #[test]
    fn normal_and_forced_share_the_same_admission_gate() {
        backend::reset(vec![Ok(())], vec![Ok(())], vec![done()]);
        let mut normal = settle(&DEVICE);
        discover(&mut normal);
        assert!(normal.admit(false));
        assert!(normal.flow.settling);
        assert!(!normal.flow.forced);

        backend::reset(vec![Ok(())], vec![Ok(())], vec![done()]);
        let mut forced = settle(&DEVICE);
        discover(&mut forced);
        assert!(forced.admit(true));
        assert!(forced.flow.settling);
        assert!(forced.flow.forced);
    }
}

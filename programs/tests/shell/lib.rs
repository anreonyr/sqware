#![allow(dead_code, unused_imports)]
extern crate alloc;
extern crate self as env;
extern crate self as ipc;
pub use abi::{MailCondition, Wait, mail};
extern crate self as pipe_client;
pub use stream::Direction;
pub struct Port;
pub enum Read {
    Bytes(usize),
    Eof,
    Pending,
}
pub enum Write {
    Bytes(usize),
    Pending,
}
impl Port {
    pub fn direction(&self) -> Direction {
        Direction::Read
    }
    pub fn token(&self) -> abi::PieToken {
        abi::PieToken::NONE
    }
    pub fn close(&mut self) -> Result<(), pipe_api::Fail> {
        Ok(())
    }
    pub fn read(&mut self, _: &mut [u8], _: Wait) -> Result<Read, pipe_api::Fail> {
        unreachable!("shared mapping belongs to QEMU tests")
    }
    pub fn write(&mut self, _: &[u8], _: Wait) -> Result<Write, pipe_api::Fail> {
        unreachable!("shared mapping belongs to QEMU tests")
    }
}
pub mod chrono {
    pub fn clock() -> u64 {
        0
    }
}
pub mod time {
    pub fn deadline(wait: crate::Wait) -> u64 {
        match wait {
            crate::Wait::Forever => u64::MAX,
            crate::Wait::AtMost(ms) => ms as u64,
        }
    }
}
mod adapt;
#[cfg(test)]
mod contracts;
#[path = "../../src/user/shell/core/mod.rs"]
mod core;
#[cfg(test)]
mod host;
mod native;
#[path = "../../src/service/pipe/core.rs"]
mod pipe;
#[cfg(test)]
mod tests {
    use super::core::*;
    use std::collections::BTreeMap;
    fn command(id: u64, ports: &[(&str, Direction)]) -> Command {
        Command {
            id: CommandId(id),
            image: "test".into(),
            args: vec![],
            ports: ports
                .iter()
                .map(|(name, direction)| PortDecl {
                    name: name.to_string(),
                    direction: *direction,
                })
                .collect(),
        }
    }
    fn end(id: u64, name: &str) -> Endpoint {
        Endpoint::Command {
            id: CommandId(id),
            port: name.into(),
        }
    }
    #[test]
    fn arbitrary_ports_require_explicit_unique_connections() {
        let cmds = vec![
            command(1, &[("records", Direction::Write)]),
            command(2, &[("source", Direction::Read)]),
        ];
        let link = Link {
            source: end(1, "records"),
            sink: end(2, "source"),
        };
        assert!(Plan::connect(cmds.clone(), vec![link.clone()], |_| None).is_ok());
        assert_eq!(
            Plan::connect(cmds.clone(), vec![], |_| None).unwrap_err(),
            PlanFail::Missing
        );
        assert_eq!(
            Plan::connect(cmds.clone(), vec![link.clone(), link], |_| None).unwrap_err(),
            PlanFail::Duplicate
        );
        assert_eq!(
            Plan::connect(
                cmds,
                vec![Link {
                    source: end(2, "source"),
                    sink: end(1, "records")
                }],
                |_| None
            )
            .unwrap_err(),
            PlanFail::Direction
        );
        assert!(Plan::connect(vec![command(3, &[])], vec![], |_| None).is_ok());
    }
    #[test]
    fn cycles_and_duplicated_commands_are_rejected() {
        let cmds = vec![
            command(1, &[("a", Direction::Read), ("b", Direction::Write)]),
            command(2, &[("a", Direction::Read), ("b", Direction::Write)]),
        ];
        assert_eq!(
            Plan::connect(
                cmds.clone(),
                vec![
                    Link {
                        source: end(1, "b"),
                        sink: end(2, "a")
                    },
                    Link {
                        source: end(2, "b"),
                        sink: end(1, "a")
                    }
                ],
                |_| None
            )
            .unwrap_err(),
            PlanFail::Cycle
        );
        assert_eq!(
            Plan::connect(vec![cmds[0].clone(), cmds[0].clone()], vec![], |_| None).unwrap_err(),
            PlanFail::Duplicate
        );
    }
    #[test]
    fn external_port_directions_are_flow_directions() {
        let ports = BTreeMap::from([(10, Direction::Read), (11, Direction::Write)]);
        let cmds = vec![command(
            1,
            &[("take", Direction::Read), ("give", Direction::Write)],
        )];
        let links = vec![
            Link {
                source: Endpoint::Port { id: 10 },
                sink: end(1, "take"),
            },
            Link {
                source: end(1, "give"),
                sink: Endpoint::Port { id: 11 },
            },
        ];
        assert!(Plan::connect(cmds, links, |id| ports.get(&id).copied()).is_ok());
    }
    #[test]
    fn partial_pause_is_not_reported_as_paused() {
        let mut job = Job::prepare(JobId(1), 2);
        job.prepared().unwrap();
        job.start().unwrap();
        job.acknowledge(0).unwrap();
        assert_eq!(job.status, JobStatus::Starting);
        job.acknowledge(1).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        job.pause(PauseReason::BackgroundRead).unwrap();
        job.acknowledge(0).unwrap();
        assert_eq!(job.status, JobStatus::Pausing);
        job.acknowledge(1).unwrap();
        assert!(matches!(
            job.event(),
            JobEvent::Paused(PauseReason::BackgroundRead)
        ));
        job.resume().unwrap();
        job.acknowledge(0).unwrap();
        job.acknowledge(1).unwrap();
        assert_eq!(job.status, JobStatus::Running);
    }
    #[test]
    fn failure_does_not_cancel_peers_and_completion_waits_for_draining() {
        let mut job = Job::prepare(JobId(1), 2);
        job.prepared().unwrap();
        job.start().unwrap();
        job.acknowledge(0).unwrap();
        job.acknowledge(1).unwrap();
        job.complete(0, 7).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        job.complete(1, 0).unwrap();
        assert_eq!(job.status, JobStatus::Running);
        job.drained();
        assert_eq!(job.status, JobStatus::Completed);
        assert!(
            matches!(job.event(), JobEvent::Completed(results) if results == vec![MemberResult::Exited(7), MemberResult::Exited(0)])
        );
    }
    #[test]
    fn cancellation_and_start_failure_wait_for_reclamation() {
        let mut job = Job::prepare(JobId(1), 2);
        job.prepared().unwrap();
        job.start().unwrap();
        job.fail();
        job.complete(0, 1).unwrap();
        job.drained();
        assert_eq!(job.status, JobStatus::Cancelling);
        job.complete(1, 2).unwrap();
        assert_eq!(job.status, JobStatus::Failed);
    }
    #[test]
    fn pipe_registry_authorizes_owner_and_keeps_roles_exclusive() {
        let mut ledger = super::pipe::Ledger::default();
        let id = ledger.create(1, 4).unwrap();
        assert!(ledger.bind(id, 2, Direction::Read, 3).is_err());
        ledger.bind(id, 1, Direction::Read, 3).unwrap();
        assert!(ledger.bind(id, 1, Direction::Read, 3).is_err());
        assert!(ledger.bind(id, 1, Direction::Read, 4).is_err());
        ledger.bind(id, 1, Direction::Write, 4).unwrap();
        ledger.release(id);
        assert!(ledger.record(id).is_err());
    }
}

use crate::{
    Mark, MailFail, Permission, PieToken, TaskId, Wait,
    raw::Hole, reply::{Reply, ReplyError}, capability::Capability,
    test_backend::{self, Event, Op},
};

fn peer() -> TaskId { TaskId::new(7) }

#[test]
fn dropping_capability_releases_without_sealing() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("owned")).unwrap();
    let token = capability.token();
    drop(capability);
    assert_eq!(test_backend::events(), [
        Event::Unseal(token, Mark::of("owned")),
        Event::Release(token),
    ]);
}

#[test]
fn dropping_loan_revokes_only_its_remote_capability() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    let loan = capability.grant(peer(), Permission::STORE, Mark::of("remote")).unwrap();
    let remote = loan.remote();
    drop(loan);
    assert_eq!(test_backend::events(), [
        Event::Unseal(source, Mark::of("source")),
        Event::Accord(source, peer(), Permission::STORE, Mark::of("remote"), remote),
        Event::Revoke(peer(), remote),
    ]);
    drop(capability);
    assert_eq!(test_backend::events().last(), Some(&Event::Release(source)));
}

#[test]
fn failed_loan_creates_no_revoke_responsibility() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    test_backend::fail_next(Op::Accord);
    assert!(capability.grant(peer(), Permission::STORE, Mark::NONE).is_err());
    drop(capability);
    let events = test_backend::events();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[1], Event::Accord(..)));
    assert_eq!(events[2], Event::Release(source));
}

#[test]
fn borrowed_loan_is_revoked_before_source_release() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    let loan = capability.grant(peer(), Permission::STORE, Mark::NONE).unwrap();
    let remote = loan.remote();
    drop(loan);
    drop(capability);
    let events = test_backend::events();
    assert_eq!(events[2], Event::Revoke(peer(), remote));
    assert_eq!(events[3], Event::Release(source));
}

#[test]
fn explicit_cleanup_errors_are_returned_without_retry() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    test_backend::fail_next(Op::Release);
    assert!(capability.release().is_err());
    assert_eq!(test_backend::events().iter().filter(|e| **e == Event::Release(source)).count(), 1);

    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let loan = capability.grant(peer(), Permission::STORE, Mark::NONE).unwrap();
    let remote = loan.remote();
    test_backend::fail_next(Op::Revoke);
    assert!(loan.revoke().is_err());
    drop(capability);
    let events = test_backend::events();
    assert_eq!(events.iter().filter(|e| **e == Event::Revoke(peer(), remote)).count(), 1);
}

#[test]
fn reply_checks_peer_and_seals_then_releases_on_drop() {
    test_backend::reset();
    let reply = Reply::open(peer(), Mark::of("back")).unwrap();
    let local = match test_backend::events()[0] { Event::Unseal(token, _) => token, _ => unreachable!() };
    let mut buffer = [0; 8];
    test_backend::queue_message(b"wrong", TaskId::new(9));
    let denied = reply.pull(&mut buffer, Wait::POLL).unwrap_err();
    assert_eq!(denied, ReplyError::WrongSource);
    test_backend::queue_message(b"right", peer());
    assert_eq!(reply.pull(&mut buffer, Wait::POLL).unwrap(), b"right");
    drop(reply);
    let events = test_backend::events();
    assert_eq!(events[events.len() - 2], Event::Seal(local));
    assert_eq!(events[events.len() - 1], Event::Release(local));
}

#[test]
fn reply_grant_failure_still_seals_and_releases() {
    test_backend::reset();
    let mut reply = Reply::open(peer(), Mark::of("claim-back")).unwrap();
    test_backend::fail_next(Op::Accord);
    assert!(reply.grant().is_err());
    drop(reply);
    let events = test_backend::events();
    let local = match events[0] { Event::Unseal(token, _) => token, _ => unreachable!() };
    assert!(matches!(events[1], Event::Accord(..)));
    assert_eq!(events[2], Event::Seal(local));
    assert_eq!(events[3], Event::Release(local));
    assert!(!events.iter().any(|event| matches!(event, Event::Revoke(..))));
}

#[test]
fn native_receive_denial_is_distinct_from_wrong_source() {
    test_backend::reset();
    let reply = Reply::open(peer(), Mark::of("back")).unwrap();
    test_backend::queue_message(b"oversized", peer());
    assert_eq!(reply.pull(&mut [0; 1], Wait::POLL), Err(ReplyError::Mail(MailFail::Denied)));
}

#[test]
fn duplicate_reply_grant_is_rejected_and_cleans_up_once() {
    test_backend::reset();
    let mut reply = Reply::open(peer(), Mark::of("back")).unwrap();
    let _remote = reply.grant().unwrap();
    assert!(reply.grant().is_err());
    drop(reply);
    let events = test_backend::events();
    let local = match events[0] { Event::Unseal(token, _) => token, _ => unreachable!() };
    let remote = match events[1] { Event::Accord(_, _, _, _, token) => token, _ => unreachable!() };
    assert_eq!(events[2..], [Event::Revoke(peer(), remote), Event::Seal(local), Event::Release(local)]);
}

#[test]
fn raw_pull_path_is_the_reply_data_path() {
    test_backend::reset();
    let token = PieToken::mint(42);
    test_backend::queue_message(b"raw", peer());
    let mut buffer = [0; 8];
    assert_eq!(Hole::from_raw(token).pull(&mut buffer, Wait::POLL).unwrap(), (3, peer()));
    assert_eq!(&buffer[..3], b"raw");
}

#[test]
fn cleanup_keeps_retrying_busy_and_yields_before_completion() {
    for op in [Op::Release, Op::Revoke] {
        test_backend::reset();
        let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
        let loan = capability.grant(peer(), Permission::STORE, Mark::NONE).unwrap();
        test_backend::busy_for(op, 12);
        drop(loan);
        drop(capability);
        let events = test_backend::events();
        assert_eq!(events.iter().filter(|event| matches!(event, Event::Starve)).count(), 12);
        let attempts = events.iter().filter(|event| match op {
            Op::Release => matches!(event, Event::Release(_)),
            Op::Revoke => matches!(event, Event::Revoke(_, _)),
            _ => unreachable!(),
        }).count();
        assert_eq!(attempts, 13);
    }
}
#[test]
fn wait_budget_includes_the_first_kernel_wait() {
    crate::test_backend::reset();
    let token = crate::test_backend::token();
    assert!(!crate::raw::Hole::from_raw(token).wait(crate::MailCondition::Pull, crate::Wait::AtMost(25)).unwrap());
    assert_eq!(crate::test_backend::now(), 25_000_000);
    assert_eq!(crate::test_backend::events(), [crate::test_backend::Event::Wait(token, crate::MailCondition::Pull, crate::Wait::AtMost(25))]);
}

#[test]
fn receive_policy_keeps_or_discards_only_the_oversized_head() {
    crate::test_backend::reset();
    let token = crate::test_backend::token();
    let sender = crate::TaskId::new(7);
    crate::test_backend::queue_message(&[1; 5], sender);
    crate::test_backend::queue_message(&[2; 2], sender);
    let hole = crate::raw::Hole::from_raw(token);
    let mut buf = [0; 2];
    assert!(hole.pull(&mut buf, crate::Wait::POLL).is_err_and(|e| e.source == crate::MailFail::Denied));
    assert_eq!(hole.pull_with(&mut buf, crate::Wait::POLL, crate::Oversize::Discard).unwrap(),
        crate::PullOutcome::Discarded { len: 5, sender });
    assert_eq!(hole.pull_with(&mut buf, crate::Wait::POLL, crate::Oversize::Discard).unwrap(),
        crate::PullOutcome::Received { len: 2, sender });
    assert_eq!(buf, [2; 2]);
}

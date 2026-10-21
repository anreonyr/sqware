use crate::{
    GrantRange, HoleFail, Mark, Permission, PieToken, TaskId, Wait,
    capability::Capability,
    raw::Hole,
    reply::{Reply, ReplyError},
    test_backend::{self, Event, Op},
};

fn peer() -> TaskId {
    TaskId::new(7)
}

#[test]
fn dropping_capability_releases_without_sealing() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("owned")).unwrap();
    let token = capability.token();
    drop(capability);
    assert_eq!(
        test_backend::events(),
        [
            Event::Unseal(token, Mark::of("owned")),
            Event::Release(token),
        ]
    );
}

#[test]
fn dropping_loan_revokes_only_its_remote_capability() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    let loan = capability
        .grant(peer(), Permission::STORE, Mark::of("remote"))
        .unwrap();
    let remote = loan.remote();
    drop(loan);
    assert_eq!(
        test_backend::events(),
        [
            Event::Unseal(source, Mark::of("source")),
            Event::Accord(
                source,
                peer(),
                Permission::STORE,
                Mark::of("remote"),
                remote
            ),
            Event::Revoke(peer(), remote),
        ]
    );
    drop(capability);
    assert_eq!(test_backend::events().last(), Some(&Event::Release(source)));
}

#[test]
fn derived_capability_grants_its_own_token_with_an_explicit_range() {
    test_backend::reset();
    let source = PieToken::mint(9);
    let range = GrantRange::Pages {
        offset: 4,
        count: 4,
    };
    let derived = Capability::derive(
        source,
        Permission::FETCH | Permission::STORE | Permission::VEST,
        Mark::of("child"),
        range,
    )
    .unwrap();
    let child = derived.token();
    assert_eq!(
        test_backend::events()[0],
        Event::Derive(
            source,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            Mark::of("child"),
            range,
            child
        )
    );
    let loan = derived
        .grant(peer(), Permission::STORE, Mark::of("bar"))
        .unwrap();
    let remote = loan.remote();
    assert_eq!(
        test_backend::events()[1],
        Event::Accord(child, peer(), Permission::STORE, Mark::of("bar"), remote)
    );
    drop(loan);
    assert_eq!(
        test_backend::events().last(),
        Some(&Event::Revoke(peer(), remote))
    );
    drop(derived);
    assert_eq!(test_backend::events().last(), Some(&Event::Release(child)));
}

#[test]
fn failed_derivation_never_releases_the_borrowed_source() {
    test_backend::reset();
    let source = PieToken::mint(9);
    test_backend::fail_next(Op::Derive);
    assert!(Capability::derive(source, Permission::FETCH, Mark::NONE, GrantRange::Inherit).is_err());
    assert!(matches!(test_backend::events().as_slice(), [Event::Derive(..)]));
    let derived = Capability::derive(source, Permission::FETCH, Mark::NONE, GrantRange::Inherit)
        .unwrap();
    let child = derived.token();
    drop(derived);
    assert_eq!(test_backend::events().last(), Some(&Event::Release(child)));
    assert!(!test_backend::events().contains(&Event::Release(source)));
}

#[test]
fn failed_loan_creates_no_revoke_responsibility() {
    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    test_backend::fail_next(Op::Accord);
    assert!(
        capability
            .grant(peer(), Permission::STORE, Mark::NONE)
            .is_err()
    );
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
    let loan = capability
        .grant(peer(), Permission::STORE, Mark::NONE)
        .unwrap();
    let remote = loan.remote();
    drop(loan);
    drop(capability);
    let events = test_backend::events();
    assert_eq!(events[2], Event::Revoke(peer(), remote));
    assert_eq!(events[3], Event::Release(source));
}

#[test]
fn failed_explicit_cleanup_retries_on_the_same_handle() {
    test_backend::reset();
    let mut capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    test_backend::fail_next(Op::Release);
    assert!(capability.release().is_err());
    capability.release().unwrap();
    capability.release().unwrap();
    drop(capability);
    let releases = test_backend::events()
        .into_iter()
        .filter(|event| *event == Event::Release(source))
        .count();
    assert_eq!(releases, 2, "failure then success, with no drop retry");

    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let mut loan = capability
        .grant(peer(), Permission::STORE, Mark::NONE)
        .unwrap();
    let remote = loan.remote();
    test_backend::fail_next(Op::Revoke);
    assert!(loan.revoke().is_err());
    loan.revoke().unwrap();
    loan.revoke().unwrap();
    drop(loan);
    let events = test_backend::events();
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == Event::Revoke(peer(), remote))
            .count(),
        2,
        "failure then success on the same peer and token"
    );
    drop(capability);
}

#[test]
fn failed_cleanup_leaves_drop_responsibility() {
    test_backend::reset();
    let mut capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let source = capability.token();
    test_backend::fail_next(Op::Release);
    assert!(capability.release().is_err());
    drop(capability);
    assert_eq!(
        test_backend::events()
            .iter()
            .filter(|event| **event == Event::Release(source))
            .count(),
        2,
        "drop retries the retained token"
    );

    test_backend::reset();
    let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
    let mut loan = capability
        .grant(peer(), Permission::STORE, Mark::NONE)
        .unwrap();
    let remote = loan.remote();
    test_backend::fail_next(Op::Revoke);
    assert!(loan.revoke().is_err());
    drop(loan);
    drop(capability);
    assert_eq!(
        test_backend::events()
            .iter()
            .filter(|event| **event == Event::Revoke(peer(), remote))
            .count(),
        2,
        "drop retries the retained peer and token"
    );
}

#[test]
fn reply_checks_peer_and_seals_then_releases_on_drop() {
    test_backend::reset();
    let reply = Reply::open(peer(), Mark::of("back")).unwrap();
    let local = match test_backend::events()[0] {
        Event::Unseal(token, _) => token,
        _ => unreachable!(),
    };
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
    let local = match events[0] {
        Event::Unseal(token, _) => token,
        _ => unreachable!(),
    };
    assert!(matches!(events[1], Event::Accord(..)));
    assert_eq!(events[2], Event::Seal(local));
    assert_eq!(events[3], Event::Release(local));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::Revoke(..)))
    );
}

#[test]
fn native_receive_denial_is_distinct_from_wrong_source() {
    test_backend::reset();
    let reply = Reply::open(peer(), Mark::of("back")).unwrap();
    test_backend::queue_message(b"oversized", peer());
    assert_eq!(
        reply.pull(&mut [0; 1], Wait::POLL),
        Err(ReplyError::Mail(HoleFail::Denied))
    );
}

#[test]
fn duplicate_reply_grant_is_rejected_and_cleans_up_once() {
    test_backend::reset();
    let mut reply = Reply::open(peer(), Mark::of("back")).unwrap();
    let _remote = reply.grant().unwrap();
    assert!(reply.grant().is_err());
    drop(reply);
    let events = test_backend::events();
    let local = match events[0] {
        Event::Unseal(token, _) => token,
        _ => unreachable!(),
    };
    let remote = match events[1] {
        Event::Accord(_, _, _, _, token) => token,
        _ => unreachable!(),
    };
    assert_eq!(
        events[2..],
        [
            Event::Revoke(peer(), remote),
            Event::Seal(local),
            Event::Release(local)
        ]
    );
}

#[test]
fn raw_pull_path_is_the_reply_data_path() {
    test_backend::reset();
    let token = PieToken::mint(42);
    test_backend::queue_message(b"raw", peer());
    let mut buffer = [0; 8];
    assert_eq!(
        Hole::from_raw(token).pull(&mut buffer, Wait::POLL).unwrap(),
        (3, peer())
    );
    assert_eq!(&buffer[..3], b"raw");
}

#[test]
fn cleanup_keeps_retrying_busy_and_yields_before_completion() {
    for op in [Op::Release, Op::Revoke] {
        test_backend::reset();
        let capability = Capability::unseal_hole(Mark::of("source")).unwrap();
        let loan = capability
            .grant(peer(), Permission::STORE, Mark::NONE)
            .unwrap();
        test_backend::busy_for(op, 12);
        drop(loan);
        drop(capability);
        let events = test_backend::events();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Starve))
                .count(),
            12
        );
        let attempts = events
            .iter()
            .filter(|event| match op {
                Op::Release => matches!(event, Event::Release(_)),
                Op::Revoke => matches!(event, Event::Revoke(_, _)),
                _ => unreachable!(),
            })
            .count();
        assert_eq!(attempts, 13);
    }
}
#[test]
fn wait_budget_includes_the_first_kernel_wait() {
    crate::test_backend::reset();
    let token = crate::test_backend::token();
    assert!(
        !crate::raw::Hole::from_raw(token)
            .wait(crate::HoleCondition::Pull, crate::Wait::AtMost(25))
            .unwrap()
    );
    assert_eq!(crate::test_backend::now(), 25_000_000);
    assert_eq!(
        crate::test_backend::events(),
        [crate::test_backend::Event::Wait(
            token,
            crate::HoleCondition::Pull,
            crate::Wait::AtMost(25)
        )]
    );
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
    assert!(
        hole.pull(&mut buf, crate::Wait::POLL)
            .is_err_and(|e| e.source == crate::HoleFail::Denied)
    );
    assert_eq!(
        hole.pull_with(&mut buf, crate::Wait::POLL, crate::Oversize::Discard)
            .unwrap(),
        crate::PullOutcome::Discarded { len: 5, sender }
    );
    assert_eq!(
        hole.pull_with(&mut buf, crate::Wait::POLL, crate::Oversize::Discard)
            .unwrap(),
        crate::PullOutcome::Received { len: 2, sender }
    );
    assert_eq!(buf, [2; 2]);
}

#[test]
fn pole_creation_hands_over_the_source_and_releases_its_reference() {
    test_backend::reset();
    let token = crate::raw::pole(crate::PAGE_SIZE, true).unwrap();
    assert_eq!(
        test_backend::events(),
        [
            Event::Acquire(crate::PAGE_SIZE),
            Event::Pole(7, true, token),
            Event::ReleaseReference(7),
        ]
    );
}

#[test]
fn failed_pole_creation_releases_only_its_source_reference() {
    test_backend::reset();
    test_backend::fail_next(Op::Unseal);
    assert!(crate::raw::pole(crate::PAGE_SIZE, false).is_err_and(|e| e.source == crate::PoleFail::Denied));
    let events = test_backend::events();
    assert!(matches!(events[1], Event::Pole(7, false, _)));
    assert_eq!(&events[2..], [Event::ReleaseReference(7),]);
}

#[test]
fn pole_allocation_failure_and_invalid_sizes_create_no_capability() {
    test_backend::reset();
    test_backend::fail_next(Op::Acquire);
    assert!(crate::raw::pole(crate::PAGE_SIZE, true).is_err_and(|e| e.source == crate::PoleFail::OoM));
    assert_eq!(test_backend::events(), [Event::Acquire(crate::PAGE_SIZE)]);
    test_backend::reset();
    assert!(crate::raw::pole(0, true).is_err_and(|e| e.source == crate::PoleFail::NotAligned));
    assert!(crate::raw::pole(crate::PAGE_SIZE - 1, true).is_err_and(|e| e.source == crate::PoleFail::NotAligned));
    assert!(test_backend::events().is_empty());
}

#[test]
fn hole_creation_failure_preserves_its_domain_without_cleanup() {
    test_backend::reset();
    test_backend::fail_next(Op::Unseal);
    assert!(Capability::unseal_hole(Mark::of("failed"))
        .is_err_and(|e| e.source == HoleFail::Denied));
    assert!(matches!(test_backend::events().as_slice(), [Event::Unseal(_, mark)] if *mark == Mark::of("failed")));
}

#[test]
fn nole_wait_rechecks_early_wakes_with_the_remaining_budget() {
    test_backend::reset();
    let mut waits = Vec::new();
    let result: abi::NoleResult<bool> = crate::raw::wait_for(Wait::AtMost(25), |step| {
        waits.push(step);
        test_backend::advance(5_000_000);
        Ok(waits.len() == 2)
    });
    assert_eq!(result.unwrap(), true);
    assert_eq!(waits, [Wait::AtMost(25), Wait::AtMost(20)]);
    assert_eq!(test_backend::now(), 10_000_000);
}

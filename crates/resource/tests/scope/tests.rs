use crate::{
    Mark, MailFail, Permission, PieToken, TaskId, Wait,
    raw::HolePie, reply::Reply, scope::Owned,
    test_backend::{self, Event, Op},
};

fn peer() -> TaskId { TaskId::new(7) }

#[test]
fn dropping_owned_releases_without_sealing() {
    test_backend::reset();
    let owned = Owned::hole(Mark::of("owned")).unwrap();
    let token = owned.token();
    drop(owned);
    assert_eq!(test_backend::events(), [
        Event::Unseal(token, Mark::of("owned")),
        Event::Release(token),
    ]);
}

#[test]
fn dropping_grant_revokes_only_its_remote_capability() {
    test_backend::reset();
    let owned = Owned::hole(Mark::of("source")).unwrap();
    let source = owned.token();
    let grant = owned.grant(peer(), Permission::STORE, Mark::of("remote")).unwrap();
    let remote = grant.remote();
    drop(grant);
    assert_eq!(test_backend::events(), [
        Event::Unseal(source, Mark::of("source")),
        Event::Accord(source, peer(), Permission::STORE, Mark::of("remote"), remote),
        Event::Revoke(peer(), remote),
    ]);
    drop(owned);
    assert_eq!(test_backend::events().last(), Some(&Event::Release(source)));
}

#[test]
fn failed_grant_creates_no_revoke_responsibility() {
    test_backend::reset();
    let owned = Owned::hole(Mark::of("source")).unwrap();
    let source = owned.token();
    test_backend::fail_next(Op::Accord);
    assert!(owned.grant(peer(), Permission::STORE, Mark::NONE).is_err());
    drop(owned);
    let events = test_backend::events();
    assert_eq!(events.len(), 3);
    assert!(matches!(events[1], Event::Accord(..)));
    assert_eq!(events[2], Event::Release(source));
}

#[test]
fn borrowed_grant_is_revoked_before_source_release() {
    test_backend::reset();
    let owned = Owned::hole(Mark::of("source")).unwrap();
    let source = owned.token();
    let grant = owned.grant(peer(), Permission::STORE, Mark::NONE).unwrap();
    let remote = grant.remote();
    drop(grant);
    drop(owned);
    let events = test_backend::events();
    assert_eq!(events[2], Event::Revoke(peer(), remote));
    assert_eq!(events[3], Event::Release(source));
}

#[test]
fn explicit_cleanup_errors_are_returned_without_retry() {
    test_backend::reset();
    let owned = Owned::hole(Mark::of("source")).unwrap();
    let source = owned.token();
    test_backend::fail_next(Op::Release);
    assert!(owned.release().is_err());
    assert_eq!(test_backend::events().iter().filter(|e| **e == Event::Release(source)).count(), 1);

    test_backend::reset();
    let owned = Owned::hole(Mark::of("source")).unwrap();
    let grant = owned.grant(peer(), Permission::STORE, Mark::NONE).unwrap();
    let remote = grant.remote();
    test_backend::fail_next(Op::Revoke);
    assert!(grant.revoke().is_err());
    drop(owned);
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
    assert_eq!(denied.source, MailFail::Denied);
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
    let reply = Reply::open(peer(), Mark::of("claim-back")).unwrap();
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
fn claim_grant_failure_cleans_up_the_previous_reply_grant() {
    test_backend::reset();
    fn build_then_claim() -> crate::PieResult<()> {
        let reply = Reply::open(peer(), Mark::of("back"))?;
        let _build = reply.grant()?;
        test_backend::fail_next(Op::Accord);
        let _claim = reply.grant()?;
        Ok(())
    }
    assert!(build_then_claim().is_err());
    let events = test_backend::events();
    let local = match events[0] { Event::Unseal(token, _) => token, _ => unreachable!() };
    let remote = match events[1] { Event::Accord(_, _, _, _, token) => token, _ => unreachable!() };
    assert_eq!(events[3..], [Event::Revoke(peer(), remote), Event::Seal(local), Event::Release(local)]);
}

#[test]
fn raw_pull_path_is_the_reply_data_path() {
    test_backend::reset();
    let token = PieToken::mint(42);
    test_backend::queue_message(b"raw", peer());
    let mut buffer = [0; 8];
    assert_eq!(HolePie::from_token(token).pull(&mut buffer, Wait::POLL).unwrap(), (3, peer()));
    assert_eq!(&buffer[..3], b"raw");
}

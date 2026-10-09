use super::*;
use loader::Face;
use system_api::loader::{Fail, Said};

fn token(raw: u64) -> PieToken {
    PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
}
fn face() -> Face {
    Face::of(token(20)).unwrap()
}
fn begin(replies: &[Result<Said, TransportFail>]) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.reset();
        state.replies.extend_from_slice(replies);
    });
}
fn snapshot() -> (Vec<CallRecord>, Vec<Event>) {
    STATE.with(|state| {
        let state = state.borrow();
        (
            state
                .calls
                .iter()
                .map(|c| CallRecord {
                    request: c.request,
                    wait: c.wait,
                })
                .collect(),
            state.events.clone(),
        )
    })
}
fn said(status: u8, team: u64, task: usize) -> Said {
    Said {
        status,
        team,
        task: TaskId::new(task),
    }
}
fn ok_build_and_claim() -> [Result<Said, TransportFail>; 2] {
    [
        Ok(said(wire::OK, 0x44, 0x55)),
        Ok(said(wire::OK, 0x44, 0x55)),
    ]
}

#[test]
fn build_and_claim_share_one_deadline_and_revoke_image_loan_after_success() {
    begin(&ok_build_and_claim());
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.elapsed_grant_ms = 5;
        state.elapsed_after_call_ms = 37;
    });
    let result = face()
        .build(token(3), 8, 12, &[4, 9], 16, Wait::AtMost(100))
        .unwrap();
    assert_eq!(
        (result.team, result.task),
        (TeamId::new(0x44), TaskId::new(0x55))
    );
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].wait, Wait::AtMost(95));
    assert_eq!(calls[1].wait, Wait::AtMost(58));
    assert!(
        matches!(calls[0].request, Captured::Build { offset: 8, len: 12, stack: 16, count: 2, args, .. } if args[..2] == [4, 9])
    );
    assert!(matches!(calls[1].request, Captured::Claim { task, .. } if task == TaskId::new(0x55)));
    assert_eq!(events.len(), 2);
    assert!(
        matches!(events[0], Event::Grant(source, peer, permission, mark, remote) if source == token(3) && peer == TaskId::new(7) && permission == Permission::FETCH && mark == system_api::loader::IMAGE && remote == token(0xabcd))
    );
    assert_eq!(events[1], Event::Revoke(TaskId::new(7), token(0xabcd)));
}

#[test]
fn grant_failure_stops_before_build_and_has_no_revoke() {
    begin(&[]);
    STATE.with(|state| state.borrow_mut().fail_grant = true);
    assert!(matches!(
        face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)),
        Err(Fail::Denied)
    ));
    let (calls, events) = snapshot();
    assert!(calls.is_empty());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::Grant(..)));
}

#[test]
fn build_transport_failure_does_not_retry_and_revokes_image_loan() {
    begin(&[Err(TransportFail::Send)]);
    assert!(matches!(
        face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)),
        Err(Fail::Bad)
    ));
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        events.last(),
        Some(&Event::Revoke(TaskId::new(7), token(0xabcd)))
    );
}

#[test]
fn build_rejection_skips_claim_and_revokes_image_loan() {
    begin(&[Ok(said(
        system_api::loader::fail_to_code(Some(Fail::BadImage)),
        0,
        0,
    ))]);
    assert!(matches!(
        face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)),
        Err(Fail::BadImage)
    ));
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].request, Captured::Build { .. }));
    assert_eq!(
        events.last(),
        Some(&Event::Revoke(TaskId::new(7), token(0xabcd)))
    );
}

#[test]
fn claim_transport_failure_uses_remaining_budget_without_retry() {
    begin(&[Ok(said(wire::OK, 0x44, 0x55)), Err(TransportFail::Receive)]);
    STATE.with(|state| state.borrow_mut().elapsed_after_call_ms = 8);
    assert!(matches!(
        face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)),
        Err(Fail::Bad)
    ));
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].wait, Wait::AtMost(10));
    assert_eq!(calls[1].wait, Wait::AtMost(2));
    assert!(matches!(calls[1].request, Captured::Claim { .. }));
    assert_eq!(
        events.last(),
        Some(&Event::Revoke(TaskId::new(7), token(0xabcd)))
    );
}

#[test]
fn claim_rejection_or_mismatched_identity_fails_and_revokes_image_loan() {
    for claim_reply in [
        said(system_api::loader::fail_to_code(Some(Fail::NotReady)), 0, 0),
        said(wire::OK, 0x45, 0x55),
    ] {
        begin(&[Ok(said(wire::OK, 0x44, 0x55)), Ok(claim_reply)]);
        assert!(
            matches!(face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)), Err(fail) if fail == (if claim_reply.status == wire::OK { Fail::Bad } else { Fail::NotReady }))
        );
        let (calls, events) = snapshot();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            events.last(),
            Some(&Event::Revoke(TaskId::new(7), token(0xabcd)))
        );
    }
}

#[test]
fn invalid_build_arguments_are_rejected_before_granting() {
    begin(&[]);
    assert!(matches!(
        face().build(token(3), 0, 0, &[], 0, Wait::POLL),
        Err(Fail::Bad)
    ));
    assert!(matches!(
        face().build(
            token(3),
            0,
            1,
            &[0; system_api::loader::MAX_ARGS + 1],
            0,
            Wait::POLL
        ),
        Err(Fail::Bad)
    ));
    assert!(snapshot().1.is_empty());
}

#[test]
fn build_receive_failure_before_claim_releases_the_granted_image() {
    begin(&[Err(TransportFail::Receive)]);
    assert!(matches!(
        face().build(token(3), 0, 1, &[], 0, Wait::AtMost(10)),
        Err(Fail::Bad)
    ));
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].request, Captured::Build { .. }));
    assert_eq!(
        events.last(),
        Some(&Event::Revoke(TaskId::new(7), token(0xabcd)))
    );
}

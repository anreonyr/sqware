use super::*;
use loader::Face;
use system_api::loader::{Fail, Said};

fn token(raw: u64) -> PieToken {
    PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
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
                .map(|call| CallRecord {
                    request: call.request,
                    wait: call.wait,
                })
                .collect(),
            state.events.clone(),
        )
    })
}
fn said(status: u8, team: u64, task: usize) -> Said {
    Said {
        status,
        state: if status == wire::OK {
            system_api::control::construction::COMPLETED
        } else {
            system_api::control::construction::FAILED
        },
        team,
        task: TaskId::new(task),
        startup: system_api::loader::StartupMetadata {
            resolve: PieToken::NONE,
        },
    }
}

#[test]
fn claim_sends_only_for_the_authorized_native_claim_face() {
    begin(&[Ok(said(wire::OK, 0x44, 0x55))]);
    let built = Face::of(token(20))
        .unwrap()
        .claim(TaskId::new(0x55), Wait::AtMost(40))
        .unwrap();
    assert_eq!(
        (built.team, built.task),
        (TeamId::new(0x44), TaskId::new(0x55))
    );
    let (calls, events) = snapshot();
    assert_eq!(calls.len(), 1);
    assert!(
        matches!(calls[0].request, Captured::Claim { task, back } if task == TaskId::new(0x55) && back == token(0x5000))
    );
    assert_eq!(calls[0].wait, Wait::AtMost(40));
    assert!(events.is_empty());
}

#[test]
fn claim_reply_must_match_the_requested_instance_and_success_shape() {
    for response in [
        said(system_api::loader::fail_to_code(Some(Fail::NotReady)), 0, 0),
        said(wire::OK, 0x44, 0x56),
        said(wire::OK, 0, 0x55),
    ] {
        begin(&[Ok(response)]);
        assert!(matches!(
            Face::of(token(20))
                .unwrap()
                .claim(TaskId::new(0x55), Wait::POLL),
            Err(Fail::NotReady)
        ));
        assert_eq!(snapshot().0.len(), 1);
    }
}

#[test]
fn claim_transport_failure_is_not_retried() {
    begin(&[Err(TransportFail::Receive)]);
    assert!(matches!(
        Face::of(token(20))
            .unwrap()
            .claim(TaskId::new(0x55), Wait::AtMost(12)),
        Err(Fail::Unknown)
    ));
    assert_eq!(snapshot().0.len(), 1);
}

#[test]
fn claim_face_accepts_only_the_actual_providers_store_only_system_claim_hole() {
    use env::{PieInfo, PieKind};
    use system_api::loader::Grant;
    begin(&[]);
    let token = token(20);
    assert!(Face::of(token).is_ok());

    let cases = [
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(9),
            vestor: TaskId::new(9),
            mark: Grant::Claim.mark(),
            alive: false,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(0),
            vestor: TaskId::new(9),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(7),
            vestor: TaskId::new(7),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(9),
            vestor: TaskId::new(7),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(10),
            vestor: TaskId::new(10),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Pole,
            permission: Permission::STORE,
            owner: TaskId::new(9),
            vestor: TaskId::new(9),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::FETCH,
            owner: TaskId::new(9),
            vestor: TaskId::new(9),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::FETCH | Permission::STORE,
            owner: TaskId::new(9),
            vestor: TaskId::new(9),
            mark: Grant::Claim.mark(),
            alive: true,
        },
        PieInfo {
            token,
            kind: PieKind::Hole,
            permission: Permission::STORE,
            owner: TaskId::new(9),
            vestor: TaskId::new(9),
            mark: system_api::loader::IMAGE,
            alive: true,
        },
    ];
    for info in cases {
        STATE.with(|state| state.borrow_mut().inspected = Some(info));
        assert!(matches!(Face::of(token), Err(Fail::Denied)));
    }
    STATE.with(|state| state.borrow_mut().inspected = None);
    STATE.with(|state| state.borrow_mut().provider_peer = TaskId::new(10));
    assert!(matches!(Face::of(token), Err(Fail::Denied)));
    let (calls, events) = snapshot();
    assert!(calls.is_empty() && events.is_empty());
}

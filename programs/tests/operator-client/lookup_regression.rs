use ipc_exchange_fixture::{
    raw,
    session::{Endpoint, Session},
    Contract, Number, PieToken, TaskId, Wait,
};
use system_api::operator::Fail;

#[path = "../../src/system/client/src/operator/lookup.rs"]
mod lookup;

struct ContractNum;

impl Contract for ContractNum {
    type Request = Number;
    type Response = Number;
}

#[test]
fn finite_reply_timeout_then_existing_path_lookup_never_retries_closed_session() {
    raw::reset();
    let s = Session::from_raw(
        Endpoint {
            sender: Some(PieToken(1)),
            receiver: PieToken(2),
        },
        PieToken(3),
        TaskId(42),
    )
    .unwrap();
    let call = |wait| {
        s.call::<ContractNum>(Number(4), wait)
            .map_err(|_| lookup::Failure::Stop(Fail::Unknown))
    };
    assert!(lookup::until(
        || call(Wait::AtMost(5)),
        |_| panic!("transport timeout must not wait for publication")
    )
    .is_err());
    // A real peer would answer this already published path; the old Session remains closed.
    raw::reply_on_next_push(vec![15], TaskId(42));
    assert!(lookup::until(
        || call(Wait::Forever),
        |_| panic!("closed Session must return immediately")
    )
    .is_err());
    assert_eq!(raw::pushes(), 1);
    assert!(!raw::alive(PieToken(2)));
}

#[test]
fn explicit_missing_path_retries_with_backoff_until_published() {
    let mut tries = 0;
    let mut pauses = Vec::new();
    let id = lookup::until(
        || {
            tries += 1;
            if tries < 4 {
                Err(lookup::Failure::Missing)
            } else {
                Ok(42)
            }
        },
        |ms| {
            pauses.push(ms);
            true
        },
    )
    .unwrap();
    assert_eq!(id, 42);
    assert_eq!(pauses, vec![10, 20, 40]);
}

#![no_std]
#![no_main]

//! probe-bound checks that malformed entry requests do not stall Operator service handling.
extern crate alloc;
extern crate programs;

use env::wire::Field;
use env::{PieToken, TaskId, Wait, unit};
use ipc::session::{Endpoint, Session};
use programs::Report;
use programs::debug;
use resource::raw::Hole;
use system_api::operator as api;
use system_api::operator::entry;
use system_client::operator;
use wire::Message;

const MS: usize = 1000;
const E_OK: usize = 0;
const E_TRIP: usize = 1;
const OK_NOTE: &str = "probe-bound: doors held against junk";
const JUNK: usize = 300;
const JUNK_OP: u8 = 1;
const SEEK_OP: u8 = 7;
const ROAD_9: &[u8] = b"a/b/c/d/e/f/g/h/i";

fn junk() -> [u8; JUNK] {
    let mut junk = [0u8; JUNK];
    junk[0] = JUNK_OP;
    junk
}

fn oversize_road() -> [u8; JUNK] {
    let mut road = [0u8; JUNK];
    road[0] = SEEK_OP;
    road[1] = ROAD_9.len() as u8;
    road[2..2 + ROAD_9.len()].copy_from_slice(ROAD_9);
    road
}

const PERMIT_OFFSET: usize = <u8 as Field>::WIDTH
    + <api::EntryId as Field>::WIDTH
    + <u8 as Field>::WIDTH
    + <u64 as Field>::WIDTH
    + <u64 as Field>::WIDTH
    + <TaskId as Field>::WIDTH
    + <u64 as Field>::WIDTH;

fn request(anchor: api::Entry, back: PieToken) -> entry::Request {
    let (anchor, root) = match anchor.target {
        api::EntryTarget::Root => (api::EntryId::new(0), 1),
        api::EntryTarget::Node(id) => (id, 0),
    };
    entry::Request {
        op: entry::BIND,
        anchor,
        root,
        id: 1,
        parent: 0,
        holder: TaskId::new(0),
        ops: 0,
        permit: api::Permit::Public,
        permit_restricted: 0,
        source: TaskId::new(0),
        source_mark: 0,
        resource: PieToken::NONE,
        resource_id: 0,
        view: api::ResourceView::new(env::Access::FETCH).unwrap().bits(),
        expected: 0,
        name: alloc::string::String::from("x"),
        scope: alloc::string::String::new(),
        back,
    }
}

fn push_request(capability: &operator::Capability, request: &entry::Request) -> Result<(), ()> {
    let mut bytes = [0u8; entry::Request::LEN];
    let n = request.store(&mut bytes).ok_or(())?;
    Hole::from_raw(capability.entrance())
        .push(&bytes[..n], Wait::AtMost(MS))
        .map_err(|_| ())
}

fn release_back(operator: TaskId, endpoint: Endpoint) {
    let _ = env::pie::revoke(operator, endpoint.seed());
    let _ = env::pie::release(endpoint.rx(), env::ReleaseMode::Revoke);
}

fn permit_trip(capability: &operator::Capability) -> (bool, bool, bool) {
    let anchor = capability.anchor().expect("probe-bound: capability anchor");
    let operator_task = anchor.operator;
    assert!(
        PERMIT_OFFSET + api::Permit::WIDTH <= entry::Request::LEN,
        "probe-bound: Permit field offset exceeds Request frame"
    );

    let malformed_back = ipc::session::establish::lend(operator_task, entry::AUTH_BACK)
        .expect("probe-bound: malformed native reply back");
    let malformed = request(anchor, malformed_back.seed());
    let mut frame = [0u8; entry::Request::LEN];
    let len = malformed
        .store(&mut frame)
        .expect("probe-bound: encode baseline Request");
    frame[PERMIT_OFFSET] = u8::MAX;
    let malformed_sent = Hole::from_raw(capability.entrance())
        .push(&frame[..len], Wait::AtMost(MS))
        .is_ok();
    let mut reply = [0u8; entry::Response::LEN];
    let malformed_reply = Hole::from_raw(malformed_back.rx()).pull(&mut reply, Wait::AtMost(MS));
    let malformed_rejected = malformed_reply.is_err();
    release_back(operator_task, malformed_back);

    let valid_back = ipc::session::establish::lend(operator_task, entry::AUTH_BACK)
        .expect("probe-bound: valid native reply back");
    let valid = request(anchor, valid_back.seed());
    let valid_sent = push_request(capability, &valid).is_ok();
    let valid_reply = Hole::from_raw(valid_back.rx()).pull(&mut reply, Wait::AtMost(MS));
    let valid_denied = match valid_reply {
        Ok((n, sender)) if sender == operator_task => entry::Response::fetch(&reply[..n])
            .is_some_and(|response| response.status == api::DENIED),
        _ => false,
    };
    release_back(operator_task, valid_back);

    let query_alive = capability
        .find(anchor, "x", api::Permit::Public, Wait::AtMost(MS))
        .err()
        == Some(api::Fail::Unknown);
    (
        malformed_sent && malformed_rejected,
        valid_sent && valid_denied,
        query_alive,
    )
}

#[programs::entry]
fn main() -> Report<'static> {
    let sire = unit::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-bound: no tree link");
    };
    let (tree, hedge) = (&unsafe { session.raw_link() }, unsafe {
        session.raw_talk()
    });
    let face = operator::Face::from(&session);

    let (junk_in, said_bad, after) = junk_trip(hedge, tree, &face, &junk());
    let (oversize_in, oversize_bad, oversize_after) =
        junk_trip(hedge, tree, &face, &oversize_road());
    let capability = operator::Capability::injected(0, sire, Wait::AtMost(MS))
        .expect("probe-bound: declared Operator entry capability");
    let (unknown_sent_and_rejected, valid_sent_and_denied, query_alive) = permit_trip(&capability);

    assert!(junk_in, "ordinary session junk was not accepted");
    assert!(said_bad, "ordinary session did not answer BAD for junk");
    assert!(after, "ordinary session query failed after junk");
    assert!(
        oversize_in,
        "oversized ordinary session road was not accepted"
    );
    assert!(
        oversize_bad,
        "oversized ordinary session road did not answer BAD"
    );
    assert!(
        oversize_after,
        "ordinary session query failed after oversized road"
    );
    assert!(
        unknown_sent_and_rejected,
        "unknown Permit tag was not decode-rejected without a reply"
    );
    assert!(
        valid_sent_and_denied,
        "valid Public BIND with resource NONE did not receive native semantic Denied"
    );
    assert!(
        query_alive,
        "Operator entry API query failed after malformed and denied requests"
    );

    programs::harness::probe::identity::timeout();
    Report::note(E_OK, OK_NOTE)
}

fn junk_trip(
    hedge: PieToken,
    tree: &Endpoint,
    face: &operator::Face,
    junk: &[u8],
) -> (bool, bool, bool) {
    let door = Hole::from_raw(hedge);
    let pushed = door.push(junk, Wait::AtMost(MS)).is_ok()
        && matches!(door.wait(env::HoleCondition::Empty, Wait::AtMost(MS)), Ok(true));

    let mut back = [0u8; 8];
    let pulled = Hole::from_raw(tree.rx())
        .pull(&mut back, Wait::AtMost(MS))
        .map(|(n, _)| n);
    debug!(
        "probe-bound: junk len={} pull={:?} code={}",
        junk.len(),
        pulled.as_ref().map(|n| *n).map_err(|e| e.source),
        back[0]
    );
    let said = pulled.ok();
    let bad = matches!(said, Some(1) if back[0] == api::BAD);
    let after = face.list(api::Where::Root, Wait::AtMost(MS)).is_ok();
    (pushed, bad, after)
}

fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    Report::note(E_TRIP, note)
}

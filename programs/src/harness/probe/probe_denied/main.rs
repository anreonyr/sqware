#![no_std]
#![no_main]

//! Possessing installer entries is not installer authority. Wrong-action frames
//! are sent raw so the probe exercises the server, not the client's WrongGrant guard.

extern crate programs;

use env::{PieToken, Wait};
use programs::Report;
use protocol::communication::hand::Receiver;
use protocol::communication::session::{Session, establish};
use protocol::service::identity;
use protocol::service::identity::client::{Organization, Query, SelfOps};
use protocol::service::identity::frame::{Reply, Wire};
use protocol::service::identity::{Fail, Grant, Install};
use protocol::service::operator::client as operator;
use runtime::env::mail::{self, HolePie};
use runtime::env::unit as utask;

const MS: usize = 1000;

#[programs::entry]
fn main() -> Report<'static> {
    // Close/reissue requests before reading the handshake. These are observations,
    // not evidence that an unrelated service failed to launch.
    for delay in [0, 1, 2, 0, 2, 1] {
        let request = establish::Held(
            establish::endpoint(
                utask::sire(),
                env::Mark::of(protocol::service::operator::LINK),
                Wait::POLL,
            )
            .expect("probe-denied: transient LINK"),
        );
        runtime::env::room::sleep(core::time::Duration::from_millis(delay))
            .expect("probe-denied: transient wait");
        drop(request);
    }
    let authority = programs::service::identity::bridge::authority()
        .expect("probe-denied: no Control-issued identity authority");
    let session = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS))
        .ok()
        .expect("probe-denied: no tree link");
    let tree = operator::Face::of(session);
    let query =
        Query::discover(&tree, authority, Wait::AtMost(MS)).expect("probe-denied: no query");
    let own = SelfOps::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-denied: no self actions");
    let org = Organization::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-denied: no organization actions");
    let me = utask::self_id();
    let initial = query
        .resolve(me, Wait::AtMost(MS))
        .unwrap()
        .expect("probe-denied: unbound");
    let p = initial.current.principal;
    let q = own
        .derive(p, Wait::AtMost(MS))
        .expect("probe-denied: derive failed");
    let c = org
        .found(Wait::AtMost(MS))
        .expect("probe-denied: found failed");
    assert_eq!(query.amid(q, c, Wait::AtMost(MS)), Ok(false));

    // Real installer faces, real valid identity, wrong sender: neither replacement
    // nor removal may touch the existing binding.
    for (grant, action) in [
        (
            Grant::Bind,
            Wire::Bind(me, Install::Authorized(initial.current)),
        ),
        (Grant::Unbind, Wire::Unbind(me)),
    ] {
        assert_eq!(
            raw(fetch(&tree, authority, grant), action),
            Reply::Fail(Fail::Denied)
        );
        assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(initial)));
    }

    // Resolve cannot be used as an alternate install or organization management face.
    let resolve = fetch(&tree, authority, Grant::Resolve);
    for action in [
        Wire::Bind(me, Install::Authorized(initial.current)),
        Wire::Admit(c, q),
        Wire::Derive(p),
    ] {
        assert_eq!(raw(resolve, action), Reply::Fail(Fail::Denied));
        assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(initial)));
        assert_eq!(query.amid(q, c, Wait::AtMost(MS)), Ok(false));
    }
    // The corresponding correct action remains live after malformed use.
    org.admit(c, q, Wait::AtMost(MS))
        .expect("probe-denied: legitimate admit failed");
    assert_eq!(query.amid(q, c, Wait::AtMost(MS)), Ok(true));
    Report::note(
        env::EXIT_OK,
        "probe-denied: installer and action isolation held",
    )
}

fn fetch(tree: &operator::Face, authority: env::TaskId, grant: Grant) -> PieToken {
    if matches!(grant, Grant::Bind | Grant::Unbind) {
        let entry = establish::find(authority, grant.mark())
            .expect("probe-denied: missing explicitly injected installer face");
        let (vestor, owner, mark) = mail::reserve(entry).unwrap();
        assert_eq!(vestor, utask::sire());
        assert_eq!(owner, authority);
        assert_eq!(mark, grant.mark());
        return entry;
    }
    let road = identity::DIR
        .try_join(grant.name())
        .expect("probe-denied: bad action name");
    let entry = tree
        .tile(&road, Wait::AtMost(MS))
        .unwrap()
        .token(Wait::AtMost(MS))
        .expect("probe-denied: action entry missing");
    let (_, owner, mark) = mail::reserve(entry).expect("probe-denied: entry cannot be reserved");
    assert_eq!(owner, authority);
    assert_eq!(mark, grant.mark());
    entry
}

fn raw(entry: PieToken, wire: Wire) -> Reply {
    let (back, seed) =
        establish::lend_out(entry, identity::BACK).expect("probe-denied: cannot establish reply");
    let mut frame = [0u8; identity::limits::MAX_FRAME];
    let n = wire
        .store(seed, &mut frame)
        .expect("probe-denied: action encode failed");
    HolePie::from_token(entry)
        .push(&frame[..n], Wait::AtMost(MS))
        .expect("probe-denied: action push failed");
    Receiver::<Reply>::from_token(back)
        .recv(&mut frame, Wait::AtMost(MS))
        .expect("probe-denied: no reply")
}

#![no_std]
#![no_main]

//! Possessing installer entries is not installer authority. Wrong-action frames
//! are sent raw so the probe exercises the server, not the client's WrongGrant guard.

extern crate programs;

use ::resource::raw::{Hole, reserve};
use env::unit;
use env::{PieToken, Wait};
use ipc::hand::Receiver;
use ipc::session::{Session, establish};
use programs::Report;
use system_api::identity::Fail;
use system_api::identity::Grant;
use system_api::identity::Install;
use system_api::identity::Reply;
use system_api::identity::Wire;
use system_client::identity::Organization;
use system_client::identity::Query;
use system_client::identity::SelfOps;
use system_client::operator;

const MS: usize = 1000;

#[programs::entry]
fn main() -> Report<'static> {
    // Close/reissue requests before reading the handshake. These are observations,
    // not evidence that an unrelated service failed to launch.
    for delay in [0, 1, 2, 0, 2, 1] {
        let request = establish::Held(
            establish::endpoint(unit::sire(), system_api::operator::LINK_MARK, Wait::POLL)
                .expect("probe-denied: transient LINK"),
        );
        execution::room::park(core::time::Duration::from_millis(delay))
            .expect("probe-denied: transient wait");
        drop(request);
    }
    let authority = system_client::identity::authority()
        .expect("probe-denied: no Control-issued identity authority");
    let session = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .ok()
        .expect("probe-denied: no tree link");
    let tree = operator::Face::of(session);
    let query =
        Query::discover(&tree, authority, Wait::AtMost(MS)).expect("probe-denied: no query");
    let own = SelfOps::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-denied: no self actions");
    let org = Organization::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-denied: no organization actions");
    let me = unit::self_id();
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
            .unwrap_or_else(|_| panic!("probe-denied: installer face missing or ambiguous"));
        let (vestor, owner, mark) = reserve(entry).unwrap();
        assert_eq!(vestor, unit::sire());
        assert_eq!(owner, authority);
        assert_eq!(mark, grant.mark());
        return entry;
    }
    let road = system_api::operator::Path::new(system_api::identity::DIR)
        .try_join(grant.name())
        .expect("probe-denied: bad action name");
    let entry = tree
        .tile(&road, Wait::AtMost(MS))
        .unwrap()
        .token(Wait::AtMost(MS))
        .expect("probe-denied: action entry missing");
    let (_, owner, mark) = reserve(entry).expect("probe-denied: entry cannot be reserved");
    assert_eq!(owner, authority);
    assert_eq!(mark, grant.mark());
    entry
}

fn raw(entry: PieToken, wire: Wire) -> Reply {
    let (back, seed) = establish::lend_out(entry, system_api::identity::BACK)
        .expect("probe-denied: cannot establish reply");
    let mut frame = [0u8; system_api::identity::limits::MAX_FRAME];
    let n = wire
        .store(seed, &mut frame)
        .expect("probe-denied: action encode failed");
    Hole::from_raw(entry)
        .push(&frame[..n], Wait::AtMost(MS))
        .expect("probe-denied: action push failed");
    Receiver::<Reply>::from_raw(back)
        .recv(&mut frame, Wait::AtMost(MS))
        .expect("probe-denied: no reply")
}

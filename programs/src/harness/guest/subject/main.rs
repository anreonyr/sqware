#![no_std]
#![no_main]

//! Runtime Identity acceptance: lineage, narrowing and irreversible restriction.
//! Installation belongs to Control; no self-unbind compatibility path exists.

extern crate programs;

use env::Wait;
use programs::Report;
use protocol::communication::session::Session;
use protocol::system::identity::client::{CallError, Query, SelfOps};
use protocol::system::identity::{Fail, PrincipalId, Subject};
use protocol::system::operator::client as operator;
use runtime::env::unit as utask;

const MS: usize = 1000;

#[programs::entry]
fn main() -> Report<'static> {
    let authority = programs::system::identity::bridge::authority()
        .expect("subject: no Control-issued identity authority");
    let session = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS))
        .ok()
        .expect("subject: no tree link");
    let tree = operator::Face::of(session);
    let query = Query::discover(&tree, authority, Wait::AtMost(MS))
        .expect("subject: no identity query");
    let own = SelfOps::discover(&tree, authority, Wait::AtMost(MS))
        .expect("subject: no identity self actions");
    let me = utask::self_id();
    let initial = query.resolve(me, Wait::AtMost(MS))
        .expect("subject: resolve failed").expect("subject: unbound");
    let p = initial.current.principal;
    assert_eq!(initial.current, initial.origin);
    let root = PrincipalId::root(authority);
    assert_eq!(query.sire(root, Wait::AtMost(MS)), Ok(None));
    assert_eq!(query.sire(p, Wait::AtMost(MS)), Ok(Some(root)));
    assert_eq!(query.heir(p, p, Wait::AtMost(MS)), Ok(true));

    let q = own.derive(p, Wait::AtMost(MS)).expect("subject: derive failed");
    let sibling = own.derive(p, Wait::AtMost(MS)).expect("subject: sibling derive failed");
    assert_eq!(query.heir(p, q, Wait::AtMost(MS)), Ok(true));
    assert_eq!(query.heir(q, p, Wait::AtMost(MS)), Ok(false));
    let outside = PrincipalId::new(authority, u64::MAX);
    assert!(matches!(query.heir(p, outside, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::UnknownPrincipal))));

    let sub = Subject::new(q, &[]).expect("subject: invalid sub subject");
    own.adopt(sub, Wait::AtMost(MS)).expect("subject: adopt failed");
    let adopted = query.resolve(me, Wait::AtMost(MS)).unwrap().unwrap();
    assert_eq!(adopted.origin, initial.origin);
    assert_eq!(adopted.current, sub);
    assert!(matches!(own.derive(p, Wait::AtMost(MS)), Err(CallError::Service(Fail::Denied))));
    assert!(matches!(own.adopt(initial.current, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::NotNarrower))));
    assert!(matches!(own.adopt(Subject::new(sibling, &[]).unwrap(), Wait::AtMost(MS)),
        Err(CallError::Service(Fail::NotNarrower))));
    assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(adopted)),
        "denied transitions must not change the binding");

    own.waive(Wait::AtMost(MS)).expect("subject: waive failed");
    assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(initial)));
    own.restrict(sub, Wait::AtMost(MS)).expect("subject: restrict failed");
    let restricted = query.resolve(me, Wait::AtMost(MS)).unwrap().unwrap();
    assert_eq!(restricted.origin, sub);
    assert_eq!(restricted.current, sub);
    own.waive(Wait::AtMost(MS)).expect("subject: restricted waive failed");
    assert_eq!(query.resolve(me, Wait::AtMost(MS)), Ok(Some(restricted)),
        "waive must not undo restrict");
    assert!(matches!(own.adopt(initial.current, Wait::AtMost(MS)),
        Err(CallError::Service(Fail::NotNarrower))));
    Report::note(env::EXIT_OK, "subject: lineage and narrowing held")
}

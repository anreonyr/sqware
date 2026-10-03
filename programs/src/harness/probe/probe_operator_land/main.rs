#![no_std]
#![no_main]
//! Raw mutation admission and trusted publication, using real IPC.
extern crate alloc;
extern crate programs;
use env::{Mark, Wait};
use programs::Report;
use protocol::communication::session::Session;
use protocol::system::control::publication::{Client, Scope, Target};
use protocol::system::operator::{
    EntryId, Fail, Grant, Permit, Where,
    client::{self as operator, Face, Mine},
};
use runtime::env::mail::{self, HolePie};
const MS: Wait = Wait::AtMost(3000);
#[programs::entry]
fn main() -> Report<'static> {
    let session = Session::open(
        runtime::env::unit::sire(),
        operator::granted_berth(Grant::Land),
        MS,
    )
    .unwrap_or_else(|_| panic!("no land session"));
    let face = Face::of(session);
    let land = face.rein(Grant::Land);
    let source = mail::unseal_hole(Mark::of("publication-test")).unwrap();
    assert_eq!(
        land.land(
            Where::Root,
            "idt".into(),
            source,
            Permit::Public,
            Mine::No,
            MS
        ),
        Err(Fail::Denied)
    );
    assert_eq!(land.part(Where::Root, "uit".into(), MS), Err(Fail::Denied));
    assert_eq!(land.trim(EntryId::new(usize::MAX), MS), Err(Fail::Denied));
    assert!(matches!(
        land.seek(protocol::common::path::Path::new("svc"), MS),
        Err(Fail::Denied)
    ));
    assert!(matches!(land.find(EntryId::new(0), MS), Err(Fail::Denied)));
    assert!(matches!(land.list(Where::Root, MS), Err(Fail::Denied)));
    assert!(matches!(land.name(EntryId::new(0), MS), Err(Fail::Denied)));
    let client = Client::injected().unwrap();
    let target = Target::Service {
        scope: Scope::Fixture,
        group: "operator-fixture".into(),
        name: "entry".into(),
    };
    let id = client
        .publish(target.clone(), source, Permit::Public, MS)
        .unwrap();
    assert_eq!(
        client.publish(target.clone(), source, Permit::Public, MS),
        Ok(id)
    );
    let other = mail::unseal_hole(Mark::of("publication-test")).unwrap();
    assert_eq!(
        client.publish(target.clone(), other, Permit::Public, MS),
        Err(Fail::Denied)
    );
    assert_eq!(
        client.publish(
            Target::Service {
                scope: Scope::Driver,
                group: "".into(),
                name: "rtc".into()
            },
            source,
            Permit::Public,
            MS
        ),
        Err(Fail::Denied)
    );
    assert_eq!(
        client.publish(
            Target::RuntimeResource {
                task: runtime::env::unit::sire(),
                kind: "public".into(),
                name: "forged".into()
            },
            source,
            Permit::Public,
            MS
        ),
        Err(Fail::Denied)
    );
    client.unpublish(target.clone(), MS).unwrap();
    assert!(
        mail::reserve(source).is_ok(),
        "unpublish must preserve the source resource"
    );
    HolePie::from_token(source).push(b"live", MS).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(
        HolePie::from_token(source).pull(&mut bytes, MS).unwrap().0,
        4
    );
    assert_eq!(&bytes, b"live");
    assert_ne!(
        client
            .publish(target.clone(), other, Permit::Public, MS)
            .unwrap(),
        id
    );
    client.unpublish(target, MS).unwrap();
    protocol::debug::put(
        "hierarchy: publication duplicate/conflict/retirement and raw grant denial passed",
    );
    Report::note(
        env::EXIT_OK,
        "probe-operator-land: trusted publication only",
    )
}

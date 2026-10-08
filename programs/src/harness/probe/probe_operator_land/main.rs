#![no_std]
#![no_main]
//! Raw mutation admission and trusted publication, using real IPC.
extern crate alloc;
extern crate programs;
use ::resource::raw::{Hole, reserve};
use env::pie;
use env::{Mark, Wait};
use ipc::session::Session;
use programs::Report;
use system_api::control::Scope;
use system_api::control::Target;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_api::operator::Where;
use system_client::control::publication::Client;
use system_client::operator;
use system_client::operator::Face;
use system_client::operator::Mine;
const MS: Wait = Wait::AtMost(3000);
#[programs::entry]
fn main() -> Report<'static> {
    let session = Session::open(env::unit::sire(), operator::BERTH, MS)
        .unwrap_or_else(|_| panic!("no Operator session"));
    let face = Face::of(session);
    let source = pie::unseal_hole(Mark::of("publication-test")).unwrap();
    assert_eq!(
        face.land(
            Where::Root,
            "idt".into(),
            source,
            Permit::Public,
            Mine::No,
            MS
        ),
        Err(Fail::Denied)
    );
    assert_eq!(face.part(Where::Root, "uit".into(), MS), Err(Fail::Denied));
    assert_eq!(face.trim(EntryId::new(usize::MAX), MS), Err(Fail::Denied));
    assert!(
        face.seek(system_api::operator::path::Path::new("svc"), MS)
            .is_ok()
    );
    assert!(face.list(Where::Root, MS).is_ok());
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
    let other = pie::unseal_hole(Mark::of("publication-test")).unwrap();
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
                task: env::unit::sire(),
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
        reserve(source).is_ok(),
        "unpublish must preserve the source resource"
    );
    Hole::from_raw(source).push(b"live", MS).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(Hole::from_raw(source).pull(&mut bytes, MS).unwrap().0, 4);
    assert_eq!(&bytes, b"live");
    assert_ne!(
        client
            .publish(target.clone(), other, Permit::Public, MS)
            .unwrap(),
        id
    );
    client.unpublish(target, MS).unwrap();
    programs::debug::put(
        "hierarchy: publication duplicate/conflict/retirement and raw grant denial passed",
    );
    Report::note(
        env::EXIT_OK,
        "probe-operator-land: trusted publication only",
    )
}

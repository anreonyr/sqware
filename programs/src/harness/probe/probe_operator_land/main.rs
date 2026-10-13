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
use system_api::operator::{self as operator_api, Fail, Permit, Where};
use system_client::operator;
use system_client::operator::Face;
const MS: Wait = Wait::AtMost(3000);
#[programs::entry]
fn main() -> Report<'static> {
    let session = Session::open(env::unit::sire(), operator::BERTH, MS)
        .unwrap_or_else(|_| panic!("no Operator session"));
    let face = Face::of(session);
    let source = pie::unseal(env::UnsealArgs::hole(Mark::of("publication-test"))).unwrap();
    let forged = pie::unseal(env::UnsealArgs::hole(
        system_api::operator::entry::AUTH_ENTRY,
    ))
    .unwrap();
    assert!(matches!(
        operator::Capability::direct(face.host(), forged),
        Err(Fail::Denied)
    ));
    let _ = pie::release(forged, env::ReleaseMode::Revoke);
    for key in [
        system_api::namespace::SERVICE,
        system_api::namespace::DEVICE,
        system_api::namespace::IDENTITY,
        system_api::namespace::UNIT,
    ] {
        assert!(
            face.resolve(key, MS).is_ok(),
            "logical namespace did not resolve"
        );
    }
    let service = face.resolve(system_api::namespace::SERVICE, MS).unwrap();
    assert!(
        face.seek_from(
            service,
            system_api::operator::Path::new("operator-fixture"),
            MS
        )
        .is_ok()
    );
    assert!(face.list(Where::Root, MS).is_ok());
    let client = system_client::operator::Capability::injected(0, env::unit::sire(), MS).unwrap();
    let anchor = client.anchor().unwrap();
    let unknown = system_api::operator::NamespaceKey::of("untrusted-association");
    assert_eq!(
        face.resolve(unknown, MS),
        Err(Fail::Unknown),
        "unassociated logical key must remain unknown"
    );
    assert_eq!(
        client.associate(unknown, anchor, MS),
        Err(Fail::Denied),
        "ordinary namespace capability must not create logical associations"
    );
    assert_eq!(
        face.resolve(unknown, MS),
        Err(Fail::Unknown),
        "denied association must not create a logical association"
    );
    assert_eq!(
        client.unpart(anchor, MS),
        Err(Fail::Denied),
        "a subtree owner cannot remove its own grant boundary"
    );
    let root = system_api::operator::Entry {
        operator: face.host(),
        target: system_api::operator::EntryTarget::Root,
    };
    assert_eq!(
        client.unpart(root, MS),
        Err(Fail::Denied),
        "a task-scoped capability cannot remove the System root"
    );
    let child = client
        .part(anchor, "private", operator_api::OperationId(1), MS)
        .unwrap()
        .entry;
    client.unpart(child, MS).unwrap();
    assert_eq!(
        client.unpart(child, MS),
        Err(Fail::Unknown),
        "retiring an already removed entry must not report success"
    );
    let view = operator_api::ResourceView::new(env::Access::STORE).unwrap();
    let first = client
        .bind(
            anchor,
            "entry",
            source,
            view,
            Permit::Public,
            operator_api::OperationId(2),
            MS,
        )
        .unwrap()
        .binding;
    assert_eq!(
        client
            .bind(
                anchor,
                "entry",
                source,
                view,
                Permit::Public,
                operator_api::OperationId(3),
                MS
            )
            .unwrap()
            .binding
            .entry_id(),
        first.entry_id()
    );
    let other = pie::unseal(env::UnsealArgs::hole(Mark::of("publication-test"))).unwrap();
    assert_eq!(
        client
            .bind(
                anchor,
                "entry",
                other,
                view,
                Permit::Public,
                operator_api::OperationId(4),
                MS
            )
            .map(|_| ()),
        Err(Fail::Denied)
    );
    assert_eq!(
        client
            .bind(
                service,
                "rtc",
                source,
                view,
                Permit::Public,
                operator_api::OperationId(5),
                MS
            )
            .map(|_| ()),
        Err(Fail::Denied)
    );
    client.unbind(first, MS).unwrap();
    assert_eq!(
        client.unbind(first, MS),
        Err(Fail::Unknown),
        "retiring an already removed binding must not report success"
    );
    assert!(
        reserve(source).is_ok(),
        "unbinding must preserve the source resource"
    );
    Hole::from_raw(source).push(b"live", MS).unwrap();
    let mut bytes = [0; 4];
    assert_eq!(Hole::from_raw(source).pull(&mut bytes, MS).unwrap().0, 4);
    assert_eq!(&bytes, b"live");
    let replacement = client
        .bind(
            anchor,
            "entry",
            other,
            view,
            Permit::Public,
            operator_api::OperationId(6),
            MS,
        )
        .unwrap()
        .binding;
    assert_ne!(replacement.entry_id(), first.entry_id());
    client.unbind(replacement, MS).unwrap();
    programs::debug::put(
        "hierarchy: publication duplicate/conflict/retirement and raw grant denial passed",
    );
    Report::note(
        env::EXIT_OK,
        "probe-operator-land: trusted publication only",
    )
}

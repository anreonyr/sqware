#![no_std]
#![no_main]

//! Publish a resource with real Task ownership, then exit.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face as TreeFace;

use runtime::env::mail;
use runtime::env::unit as utask;

const MS: usize = 1000;

const E_OK: usize = 0;
/// 走不下去（`bail`）那一档：**与"判据没过"是两回事**——判据没过走 panic 通道
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-lease: landed, leaving";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-lease: no tree link");
    };
    let tree = TreeFace::of(session);
    let entry = mail::unseal_hole(env::Mark::of("lease-entry")).unwrap();
    let target = protocol::system::control::publication::Target::Service {
        scope: protocol::system::control::publication::Scope::Fixture,
        group: "fixtures".into(),
        name: "lease".into(),
    };
    let id = protocol::system::control::publication::Client::injected()
        .unwrap()
        .publish(target, entry, Permit::Public, Wait::AtMost(MS))
        .unwrap();
    let cap = tree
        .tile(
            protocol::common::path::Path::new("svc/fixtures/lease"),
            Wait::AtMost(MS),
        )
        .unwrap()
        .token(Wait::AtMost(MS))
        .unwrap();
    assert_eq!(
        mail::reserve(cap).unwrap().1,
        utask::self_id(),
        "Control must preserve publisher ownership"
    );
    debug!("probe-lease: publication={}", id.get());

    return Report::note(E_OK, OK_NOTE);
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}

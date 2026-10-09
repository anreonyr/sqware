#![no_std]
#![no_main]

//! Publish a resource with real Task ownership, then exit.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use ipc::session::Session;
use programs::debug;
use system_api::operator::Permit;
use system_client::operator::Face as Face;
use system_client::operator;

use env::unit;
use env::pie;
use ::resource::raw::{reserve};

const MS: usize = 1000;

const E_OK: usize = 0;
/// 走不下去（`bail`）那一档：**与"判据没过"是两回事**——判据没过走 panic 通道
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-lease: landed, leaving";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = unit::sire();
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-lease: no tree link");
    };
    let tree = Face::of(session);
    let entry = pie::unseal_hole(env::Mark::of("lease-entry")).unwrap();
    let target = system_api::control::publication::Target::Service {
        scope: system_api::control::publication::Scope(4),
        group: "fixtures".into(),
        name: "lease".into(),
    };
    let id = system_client::control::publication::Client::injected()
        .unwrap()
        .publish(target, entry, Permit::Public, Wait::AtMost(MS))
        .unwrap();
    let cap = tree
        .tile(
            system_api::operator::path::Path::new("svc/fixtures/lease"),
            Wait::AtMost(MS),
        )
        .unwrap()
        .token(Wait::AtMost(MS))
        .unwrap();
    assert_eq!(
        reserve(cap).unwrap().1,
        unit::self_id(),
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

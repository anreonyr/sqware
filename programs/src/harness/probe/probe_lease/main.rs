#![no_std]
#![no_main]

//! Publish a resource with real Task ownership, then exit.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use ipc::session::Session;
use programs::debug;
use system_api::operator::{self as operator_api, Permit};
use system_client::operator;
use system_client::operator::Face;

use ::resource::raw::reserve;
use env::pie;
use env::unit;

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
    let entry = pie::unseal(env::UnsealArgs::hole(env::Mark::of("lease-entry"))).unwrap();
    let capability =
        system_client::operator::Capability::injected(0, sire, Wait::AtMost(MS)).unwrap();
    let anchor = capability.anchor().unwrap();
    let view = operator_api::ResourceView::new(env::Access::STORE).unwrap();
    let binding = capability
        .bind(
            anchor,
            "lease",
            entry,
            view,
            Permit::Public,
            operator_api::OperationId(1),
            Wait::AtMost(MS),
        )
        .unwrap()
        .binding;
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
    debug!("probe-lease: binding={}", binding.entry_id().get());

    return Report::note(E_OK, OK_NOTE);
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}

#![no_std]
#![no_main]

//! Verify retired operation paths, mutation admission and independent sessions.

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use env::pie;
use env::unit;
use ipc::session::Session;
use system_api::operator::Fail;
use system_api::operator::Permit;
use system_client::operator;
use system_client::operator::Face;
use system_client::operator::Mine;

const MS: usize = 1000;

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: no tree link");
    };
    let requests = session.clone();
    let tree = Face::of(session);
    reject_wrong_watch_role(&requests);
    let watch = tree
        .watch(system_api::operator::Path::ROOT, Wait::AtMost(MS))
        .expect("first owned Watch");
    drop(watch);
    let replacement = tree
        .watch(system_api::operator::Path::ROOT, Wait::AtMost(MS))
        .expect("closed Watch must permit same-path resubscription");
    drop(replacement);

    for operation in [
        "part", "land", "find", "trim", "list", "seek", "name", "watch",
    ] {
        let road =
            system_api::operator::PathBuf::try_new(&alloc::format!("svc/sys/operator/{operation}"))
                .unwrap();
        assert_eq!(tree.seek(&road, Wait::AtMost(MS)), Err(Fail::Unknown));
    }
    let existing = tree
        .pane(system_api::operator::SVC, Wait::AtMost(3000))
        .expect("probe-operator-gate: service directory")
        .id();

    let root = tree.root();
    assert!(matches!(
        root.open("idt".into(), Wait::AtMost(MS)),
        Err(Fail::Denied)
    ));
    let source = pie::unseal_hole(env::Mark::of("raw-generic")).unwrap();
    assert!(matches!(
        root.bind(
            "uit".into(),
            source,
            Permit::Public,
            Mine::No,
            Wait::AtMost(MS)
        ),
        Err(Fail::Denied)
    ));
    assert!(matches!(
        root.trim(existing, Wait::AtMost(MS)),
        Err(Fail::Denied)
    ));
    independent_sessions();
    programs::debug::put("hierarchy: generic Part/Land/Trim denied for bound Task");
    Report::note(
        env::EXIT_OK,
        "probe-operator-gate: unified sessions and retired roles",
    )
}

/// Exercise independent sessions while the task's original Operator session stays alive.
fn independent_sessions() {
    use ::resource::raw::{alive, table_size};
    let before = table_size();
    let stalled = ipc::session::Held(
        ipc::session::establish::lend(unit::sire(), system_api::operator::LINK_MARK)
            .expect("half-open LINK"),
    );
    let started = env::chrono::clock();
    let first = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .unwrap_or_else(|_| panic!("independent Operator session"));
    assert!(
        env::chrono::clock().saturating_sub(started) < 5_000_000_000,
        "half-open peer must not impose the bootstrap timeout on healthy sessions"
    );
    let first_reply = unsafe { first.raw_link() }.rx();
    let first_request = unsafe { first.raw_talk() };
    let second = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .unwrap_or_else(|_| panic!("independent Operator session"));
    assert_ne!(first_reply, unsafe { second.raw_link() }.rx());
    assert_ne!(first_request, unsafe { second.raw_talk() });
    let alias = first.clone();
    drop(first);
    assert!(alive(first_reply) && alive(first_request));
    let first = Face::of(alias);
    let second = Face::of(second);
    assert!(
        first
            .list(system_api::operator::Where::Root, Wait::AtMost(MS))
            .is_ok()
    );
    assert!(
        second
            .list(system_api::operator::Where::Root, Wait::AtMost(MS))
            .is_ok()
    );
    drop(first);
    assert!(!alive(first_reply) && !alive(first_request));
    assert!(
        second
            .list(system_api::operator::Where::Root, Wait::AtMost(MS))
            .is_ok()
    );
    let third = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS))
        .unwrap_or_else(|_| panic!("independent Operator session"));
    let third = Face::of(third);
    assert!(
        third
            .list(system_api::operator::Where::Root, Wait::AtMost(MS))
            .is_ok()
    );
    assert!(
        second
            .list(system_api::operator::Where::Root, Wait::AtMost(MS))
            .is_ok()
    );
    drop(third);
    drop(second);
    drop(stalled);
    let budget = ipc::time::Deadline::new(Wait::AtMost(MS));
    while table_size() != before && budget.remaining() != Wait::POLL {
        execution::room::park(core::time::Duration::from_millis(1)).unwrap();
    }
    assert_eq!(
        table_size(),
        before,
        "closed sessions must return the native table to baseline"
    );
    programs::debug::put("operator: same-task sessions, aliases and independent retirement passed");
}

/// A valid transferred Hole is not a Watch endpoint unless its declared role matches.
fn reject_wrong_watch_role(session: &Session) {
    let local = pie::unseal_hole(env::Mark::NONE).unwrap();
    let borrowed =
        ::resource::port::ship(local, session.host(), env::Access::STORE, env::Policy::NONE)
            .unwrap();
    let said = session
        .call::<system_api::operator::Call>(
            system_api::operator::Req::Watch {
                road: system_api::operator::Path::ROOT.to_path_buf(),
                hole: borrowed.seed(),
            },
            Wait::AtMost(MS),
        )
        .unwrap_or_else(|_| panic!("watch import refusal must produce a reply"));
    assert_eq!(said.code(), system_api::operator::DENIED);
    assert!(
        ::resource::raw::alive(local),
        "refusing Watch must preserve the caller's source"
    );
    pie::release(local).unwrap();
    assert!(
        session
            .call::<system_api::operator::Call>(
                system_api::operator::Req::List(system_api::operator::Where::Root),
                Wait::AtMost(MS),
            )
            .is_ok(),
        "invalid Watch must not poison the request session"
    );
}

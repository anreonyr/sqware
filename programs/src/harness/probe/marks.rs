//! Kernel baseline for capability marks: they label references but do not add authority.

use ::resource::raw::{Hole, reserve, table_size};
use env::{MailFail, Mark, Permission, Wait, pie, unit};

const SOURCE: Mark = Mark::of("mark-baseline-source");
const DUPLICATE: Mark = Mark::of("mark-baseline-duplicate");

pub fn acceptance() {
    assert_eq!(
        system_client::operator::BERTH.ask,
        system_api::operator::ASK_MARK
    );

    let owner = unit::self_id();
    let before = table_size();

    let source = pie::unseal_hole(SOURCE).unwrap();
    pie::narrow(source, Permission::FETCH | Permission::VEST).unwrap();
    assert_eq!(
        reserve(source).unwrap(),
        (env::TaskId::new(0), owner, SOURCE)
    );

    // Relabel a child with a real API Grant mark. The badge changes; the source and
    // the child's actual FETCH-only permission do not.
    let grant_mark = system_api::loader::Grant::Build.mark();
    let relabeled = pie::accord(source, owner, Permission::FETCH, grant_mark).unwrap();
    assert_eq!(
        reserve(source).unwrap(),
        (env::TaskId::new(0), owner, SOURCE)
    );
    assert_eq!(reserve(relabeled).unwrap(), (owner, owner, grant_mark));
    assert!(matches!(
        Hole::from_raw(relabeled).push(b"store denied", Wait::POLL),
        Err(error) if error.source == MailFail::Denied
    ));
    assert!(matches!(
        pie::accord(source, owner, Permission::FETCH | Permission::STORE, grant_mark),
        Err(error) if error.source == env::PieFail::Denied
    ));
    assert!(matches!(
        pie::accord(relabeled, owner, Permission::FETCH, grant_mark),
        Err(error) if error.source == env::PieFail::Denied
    ));

    let inherited = pie::accord(source, owner, Permission::FETCH, Mark::NONE).unwrap();
    assert_eq!(reserve(inherited).unwrap(), (owner, owner, SOURCE));

    let first = pie::unseal_hole(DUPLICATE).unwrap();
    let second = pie::unseal_hole(DUPLICATE).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        reserve(first).unwrap(),
        (env::TaskId::new(0), owner, DUPLICATE)
    );
    assert_eq!(
        reserve(second).unwrap(),
        (env::TaskId::new(0), owner, DUPLICATE)
    );

    assert_eq!(
        ipc::session::establish::find(owner, DUPLICATE),
        Err(ipc::session::establish::DiscoveryFail::Ambiguous)
    );
    assert_eq!(
        ipc::session::establish::claim(owner, DUPLICATE, Wait::AtMost(1000)),
        Err(ipc::session::establish::DiscoveryFail::Ambiguous)
    );
    pie::release(second).unwrap();
    assert_eq!(ipc::session::establish::find(owner, DUPLICATE), Ok(first));

    let unmarked = pie::unseal_hole(Mark::NONE).unwrap();
    assert_eq!(
        reserve(unmarked).unwrap(),
        (env::TaskId::new(0), owner, Mark::NONE)
    );
    Hole::from_raw(unmarked)
        .push(b"unmarked", Wait::POLL)
        .unwrap();
    let mut bytes = [0; 8];
    assert_eq!(
        Hole::from_raw(unmarked)
            .pull(&mut bytes, Wait::POLL)
            .unwrap(),
        (8, owner)
    );
    assert_eq!(&bytes, b"unmarked");

    pie::revoke(owner, relabeled).unwrap();
    pie::revoke(owner, inherited).unwrap();
    for token in [first, unmarked, source] {
        pie::seal(token).unwrap();
        pie::release(token).unwrap();
        assert!(reserve(token).is_err());
    }
    assert_eq!(table_size(), before, "mark probe leaked capability entries");
    programs::debug::put(
        "marks: relabel, NONE inheritance, duplicate owner/mark, restricted access and unmarked I/O passed",
    );
}

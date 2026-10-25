use env::{AcquireSource, Mark, MemoryFail, PAGE_SIZE, Permission, Wait};
use execution::memory::{Reference, traverse};
use resource::raw::Hole;
use wire::Message;

pub(super) fn check() {
    let system = env::unit::sire();
    let descriptor = ipc::session::establish::claim(
        system,
        system_api::supply::descriptor_mark(Mark::of("probe-lease/retention")),
        Wait::AtMost(2000),
    )
    .expect("probe-lease: retention descriptor");
    let mut bytes = system_api::supply::SupplyFrame::EMPTY;
    let (len, from) = Hole::from_raw(descriptor)
        .pull(&mut bytes, Wait::AtMost(2000))
        .expect("probe-lease: retention frame");
    assert_eq!(from, system);
    let frame = system_api::supply::SupplyFrame::fetch(&bytes[..len]).unwrap();
    assert_eq!(frame.length, 1);
    assert_eq!(frame.description[0], 1);
    let token =
        programs::driver::shared::retention::claim(system, Mark::of("probe-lease/retention"))
            .expect("probe-lease: retention supply");
    let retention = resource::raw::inspect(token).unwrap();
    assert_eq!(retention.owner, system);
    assert_ne!(retention.owner, env::unit::self_id());
    let source = resource::raw::pole(2 * PAGE_SIZE, true).unwrap();
    let range = AcquireSource::Source {
        token: source,
        offset: 0,
        count: 2,
    };
    let reduced = env::pie::accord(
        retention.token,
        env::unit::self_id(),
        Permission::FETCH,
        Mark::of("probe-lease/reduced-retention"),
    )
    .unwrap();
    assert!(matches!(Reference::acquire(range, reduced), Err(error)
        if error.source == MemoryFail::Denied));
    env::pie::release(reduced, env::ReleaseMode::Revoke).unwrap();
    let fake = env::nole::unseal().unwrap();
    assert!(matches!(Reference::acquire(range, fake), Err(error)
        if error.source == MemoryFail::Denied));
    resource::raw::release(fake).unwrap();
    let mut reference = Reference::acquire(range, retention.token)
        .expect("probe-lease: delegated retention acquire");
    let range = env::ReferenceRange::new(reference.handle().unwrap(), 0, 2).unwrap();
    let mut physical = [0; 2];
    traverse(retention.token, range, &mut physical)
        .expect("probe-lease: delegated physical export");
    assert!(
        physical
            .iter()
            .all(|page| *page != 0 && *page % PAGE_SIZE as u64 == 0)
    );
    assert_ne!(physical[0], physical[1]);
    reference.release().unwrap();
    resource::raw::release(source).unwrap();
    programs::debug::put("probe-lease: declared retention acquired, checked and released");
}

mod accessor;
mod effects;
mod platform;

use super::super::retention_core::{
    AddressRange, DomainId, DomainRange, Failure, Holds, Registration, Scope,
};
use super::{Caller, KVirt, PAGE_SIZE, pend};
use alloc::{boxed::Box, rc::Rc, sync::Arc};
use core::cell::Cell;
use env::{AcquireSource, EnvCall, MemoryCall, PieToken};

pub fn accept() {
    crate::core::work::room::scheduler::init().unwrap();
    let driver = Caller::new(false);
    let system = Caller::new(false);
    let (_, at, source) = driver.attach_alloc_reference(7 * PAGE_SIZE, 6);
    let frames = {
        let view = driver.team.space.nest.probe(KVirt::wrap(at)).unwrap();
        view.frames_in(KVirt::wrap(at), 7 * PAGE_SIZE)
            .unwrap()
            .iter()
            .map(|(_, frame)| Arc::downgrade(frame))
            .collect()
    };
    let external = Rc::new(accessor::Accessor {
        frames,
        active: Cell::new(false),
        timeout: Cell::new(false),
    });
    let root = crate::entry::call::pie::create_pole(&driver.task, source, true).unwrap();
    assert_eq!(
        driver.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let grant = PieToken::from_bytes(
        &pend::accord(
            &driver.task,
            root,
            &Arc::downgrade(&system.task),
            env::Permission::FETCH | env::Permission::VEST,
            env::Mark::NONE,
        )
        .unwrap()
        .to_le_bytes(),
    )
    .unwrap();
    let cap = crate::entry::boot::resources::grant_retention(&system.task).unwrap();
    let domain_range = DomainRange {
        domain: DomainId::new(0x301_0000),
        range: AddressRange::new(0x1000_0000, 4096).unwrap(),
    };
    let mut holds = Holds::new();
    let scope = holds
        .attach(Scope::new(
            domain_range,
            driver.task.ident.id,
            Some(Box::new(platform::Device {
                domain_range,
                accessor: external.clone(),
            })),
        ))
        .unwrap();
    let mut effects = effects::Effects { caller: &system };
    for offset in 0..7 {
        let id = holds
            .register(
                Registration {
                    scope,
                    source: AcquireSource::Source {
                        token: grant,
                        offset,
                        count: 1,
                    },
                    retention: cap.token(),
                },
                &mut effects,
            )
            .unwrap();
        holds
            .publish(id, |_| {
                external.active.set(true);
                Ok(())
            })
            .unwrap();
    }
    // The source task is dead and ordinary cleanup has begun before System receives notice.
    driver.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    pend::doom(&driver.task);
    assert!(driver.team.space.nest.translate(KVirt::wrap(at)).is_none());
    assert!(
        external.write(0x61),
        "all seven actual frames survive source Task/Pole/Pie loss"
    );
    for frame in &external.frames {
        assert_eq!(
            unsafe { frame.upgrade().unwrap().as_ptr().read_volatile() },
            0x61
        );
    }
    holds.retry(&mut effects);
    assert!(
        external.write(0x62),
        "lost death notification cannot release the frames"
    );
    holds.offline(driver.task.ident.id);
    holds.offline(driver.task.ident.id);
    external.timeout.set(true);
    assert!(holds.retry(&mut effects));
    assert_eq!(holds.occupied().next().unwrap().references, 7);
    assert_eq!(
        holds.occupied().next().unwrap().failure,
        Some(Failure::Pending)
    );
    assert!(
        external.write(0x63),
        "isolation timeout retains externally reachable RAM"
    );
    external.timeout.set(false);
    holds.retry(&mut effects);
    assert!(!external.write(0x64));
    assert_eq!(holds.occupied().count(), 0);
    assert!(
        external
            .frames
            .iter()
            .all(|frame| frame.upgrade().is_none()),
        "successful isolation releases every data/control frame, with no CPU view pin"
    );
    holds.offline(driver.task.ident.id);
    holds.retry(&mut effects);
    assert_eq!(holds.occupied().count(), 0);
}

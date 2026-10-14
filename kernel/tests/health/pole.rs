use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::table::{PteFlags, TABLE_LIVE};
use crate::core::memory::nest::{Access, RegionId, identity};
use crate::entry::call::pie::{create_pole, fail_pole_at};
use crate::core::work::mail::pole;
use crate::core::work::pend::{self, Permission};
use crate::core::work::unit::{
    space::SpaceBuilder,
    team::TeamBuilder,
};
use alloc::sync::Arc;
use alloc::vec;

pub fn lifecycle() {
    let before = TABLE_LIVE.load(::core::sync::atomic::Ordering::Relaxed);
    let space = SpaceBuilder::user().build().unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let peer = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let recipient = peer.task().hold().unwrap();
    let rw = Access::READ | Access::WRITE;
    let reservation = team
        .space
        .memory
        .allocate_low(RegionId::Normal, PAGE_SIZE)
        .unwrap();
    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    let prepared = caller.pend.creations.prepare().unwrap();
    let input = team
        .space
        .memory
        .map_frames(reservation, vec![frame], rw | Access::EXECUTE, rw)
        .unwrap();
    prepared.commit(input.view());
    let start = input.address();
    let original = input.view().active().unwrap()[0].current.bits();
    let kernel = crate::core::work::unit::team::kernel().unwrap().space.clone();
    let tables = TABLE_LIVE.load(::core::sync::atomic::Ordering::Relaxed);
    for stage in 1..=3 {
        fail_pole_at(stage);
        assert_eq!(
            create_pole(&caller, start, PAGE_SIZE, true),
            Err(env::PieFail::OoM)
        );
        assert!(caller.pend.pies.lock().is_empty());
        assert!(Arc::ptr_eq(
            &team.space.memory.backend().probe(start).unwrap(),
            input.view()
        ));
        assert_eq!(input.view().active().unwrap()[0].current.bits(), original);
        let _lease = caller.pend.creations.claim(input.view()).unwrap();
        assert_eq!(
            TABLE_LIVE.load(::core::sync::atomic::Ordering::Relaxed),
            tables
        );
    }
    identity::fail_next_shootdown(kernel.memory.asid());
    assert_eq!(
        create_pole(&caller, start, PAGE_SIZE, true),
        Err(env::PieFail::Busy)
    );
    assert!(caller.pend.pies.lock().is_empty());
    kernel.memory.retry_invalidation().unwrap();
    assert_eq!(
        TABLE_LIVE.load(::core::sync::atomic::Ordering::Relaxed),
        tables
    );
    assert_eq!(input.view().active().unwrap()[0].current.bits(), original);
    let token = create_pole(&caller, start, PAGE_SIZE, true).unwrap();
    assert_eq!(
        create_pole(&caller, start, PAGE_SIZE, true),
        Err(env::PieFail::Denied)
    );
    let snapshot = pend::locate(&caller, token).unwrap();
    let meta = snapshot.mail().pole().unwrap();
    let first = pole::open(&meta, token, &team.space, rw).unwrap();
    assert_eq!(pole::open(&meta, token, &team.space, rw).unwrap(), first);
    assert_eq!(first.0, start.as_usize());
    assert!(meta.opened(input.view()));
    let child = env::PieToken::mint(
        pend::accord(
            &caller,
            token,
            &Arc::downgrade(&recipient),
            Permission::FETCH | Permission::STORE,
            env::Mark::NONE,
        )
        .unwrap(),
    );
    let (shared, _) = pole::open(&meta, child, &peer.space, rw).unwrap();
    let peer_map = peer
        .space
        .memory
        .backend()
        .probe(VirtAddr::wrap(shared))
        .unwrap();
    assert!(meta.opened(&peer_map));
    assert_eq!(
        peer.space
            .memory
            .backend()
            .translate(VirtAddr::wrap(shared))
            .unwrap()
            .0,
        team.space.memory.backend().translate(start).unwrap().0
    );
    assert_eq!(
        create_pole(&recipient, VirtAddr::wrap(shared), PAGE_SIZE, true),
        Err(env::PieFail::Denied)
    );
    pend::reduce(&caller, token, Permission::FETCH | Permission::VEST).unwrap();
    assert!(!snapshot.permission().contains(Permission::STORE));
    assert!(
        !team
            .space
            .memory
            .backend()
            .translate(start)
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(!meta.readonly());
    assert!(!meta.exclusive());
    pend::revoke(&caller, &Arc::downgrade(&recipient), child).unwrap();
    assert!(meta.readonly());
    assert!(meta.exclusive());
    assert!(peer.space.memory.backend().translate(VirtAddr::wrap(shared)).is_none());
    pole::shut(&meta, token).unwrap();
    assert!(weak.upgrade().is_some());
    assert!(team.space.memory.backend().translate(start).is_none());
    let (reopened, _) = pole::open(&meta, token, &team.space, Access::READ).unwrap();
    assert!(team.space.memory.validate_read(reopened, PAGE_SIZE));
    assert!(!team.space.memory.validate_write(reopened, PAGE_SIZE));
    assert_eq!(
        create_pole(&caller, VirtAddr::wrap(reopened), PAGE_SIZE, true),
        Err(env::PieFail::Denied)
    );
    pend::release(&caller, token).unwrap();
    assert!(!snapshot.alive());
    drop(snapshot);
    drop(meta);
    drop(input);
    // peer_map 记录仍持有来源，CPU 翻译已经撤销。
    assert!(
        weak.upgrade().is_some(),
        "flat: the open peer map still holds the source"
    );
    drop(peer_map);
    // 最后一份来源持有消失后，帧回到分配器。
    assert!(
        weak.upgrade().is_none(),
        "flat: the last source reference releases the page"
    );
    assert!(peer.release_held(&recipient));
    drop(recipient);
    drop(peer);
    assert!(team.release_held(&caller));
    drop(caller);
    drop(team);
    drop(kernel);
    assert_eq!(
        TABLE_LIVE.load(::core::sync::atomic::Ordering::Relaxed),
        before
    );
}

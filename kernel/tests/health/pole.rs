use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
use crate::core::memory::nest::table::{PteFlags, TABLE_LIVE};
use crate::core::memory::nest::{Access, NestError, RegionId, identity};
use crate::core::work::mail::{Mail, pole};
use crate::core::work::pend::{self, Permission};
use crate::core::work::unit::{space::SpaceBuilder, team::TeamBuilder};
use crate::entry::call::pole::{create_pole, fail_pole_at};
use ::core::sync::atomic::Ordering;
use alloc::sync::Arc;
use alloc::vec;

/// 实际持有的整页读数：帧池取给块池的整页由 `block_occupied` 单记。
fn held_pages() -> usize {
    frame_occupied() - block_occupied()
}

pub fn unpublished_roots() {
    use crate::core::memory::address::{PageCount, PageOffset};
    use crate::core::work::mail::pole::PoleOperation;
    use crate::core::work::pend::SourceAuthorization;
    use crate::core::work::room::{messenger, scheduler};

    let seed = identity::Asid::try_allocate().unwrap();
    identity::Asid::release_unpublished(seed);
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let pages = held_pages();
    let sites = messenger::site_count();
    {
        let team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
            .spawn()
            .unwrap();
        let task = team.task().hold().unwrap();
        let rights = Permission::FETCH | Permission::STORE | Permission::VEST;
        let hole = crate::core::work::mail::hole::HoleMeta::try_new(task.ident.id).unwrap();
        let hole_life = Arc::downgrade(&hole);
        let pie = pend::Pie::try_new(Mail::Hole(hole), env::Mark::NONE, rights, None).unwrap();
        let token = pie.token();
        pend::insert(&task, pie).unwrap();
        assert_eq!(pend::release(&task, token), Ok(1));
        assert!(pend::locate(&task, token).is_none());
        assert!(hole_life.upgrade().is_none());
        drop(hole_life);

        let source_pages = held_pages();
        let frames: alloc::vec::Vec<_> = (0..4).map(|_| Frame::allocate().unwrap()).collect();
        let first_frame = Arc::downgrade(&frames[0]);
        let source = super::support::frames_source(frames).unwrap();
        let authorization = SourceAuthorization::new(source, Access::all(), task.ident.id).unwrap();
        let id = task
            .pend
            .references
            .insert(
                authorization.clone(),
                PageOffset::ZERO,
                PageCount::new(4).unwrap(),
                Access::all(),
                None,
                None,
            )
            .unwrap();
        let reference = env::FlatReference::from_bytes(&id.to_le_bytes()).unwrap();
        let token = create_pole(&task, reference, true).unwrap();
        drop(task.pend.references.remove(id).unwrap());
        assert_eq!(held_pages(), source_pages + 4);
        {
            let snapshot = pend::locate(&task, token).unwrap();
            let meta = snapshot.mail().pole().unwrap();
            assert!(!meta.has_maps());
            let operation = PoleOperation::acquire(&meta).unwrap();
            assert_eq!(pend::release(&task, token), Err(env::PieFail::Busy));
            assert!(pend::locate(&task, token).is_some());
            assert!(first_frame.upgrade().is_some());
            drop(operation);
        }
        assert_eq!(pend::release(&task, token), Ok(1));
        assert!(pend::locate(&task, token).is_none());
        assert!(first_frame.upgrade().is_none());
        assert!(authorization.full_range().is_none());
        assert_eq!(held_pages(), source_pages);
        drop(first_frame);
        messenger::slay(&task);
        assert!(task.reaped());
    }
    messenger::bury();
    scheduler::roster::prune_dead();
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(held_pages(), pages);
}

/// Unseal 前同源可以有多个 Attach（含 slice）。Open 只能复用**整窗且活动完整**
/// 的那一条：slice 不能顶替整窗，否则会把第 1 页当成源 0 页、或报整长却只有几个 PTE。
/// 原 slice 关系仍归撤销清理。
pub fn source_window_and_active_shape() {
    use crate::core::memory::address::{PageCount, PageOffset};
    use crate::core::memory::flat::FlatRange;
    use crate::core::work::pend::SourceAuthorization;

    fn reference_of(raw: usize) -> env::FlatReference {
        match env::MemoryCall::from_wire((2usize << 32) | 6, &[raw, 0, 0, 0, 0, 0]).unwrap() {
            env::MemoryCall::Release { reference } => reference,
            other => panic!("unexpected release: {other:?}"),
        }
    }

    let space = SpaceBuilder::user().build().unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let rw = Access::READ | Access::WRITE;
    let frames: alloc::vec::Vec<_> = (0..4).map(|_| Frame::allocate().unwrap()).collect();
    let source: FlatRange = super::support::frames_source(frames.clone()).unwrap();
    // 先 Attach 中间两页：同一份来源的 slice，不是整窗。
    let middle = source
        .range(PageOffset::new(1), PageCount::new(2).unwrap())
        .unwrap();
    let reservation =
        super::support::reserve_low(&team.space, RegionId::Normal, 2 * PAGE_SIZE).unwrap();
    let slice = team
        .space
        .nest
        .attach(reservation, middle, Access::all(), rw)
        .unwrap();

    // 这份来源的引用（整窗 4 页），并把 slice 也登在同一份来源名下。
    let authorization =
        SourceAuthorization::new(source.clone(), Access::all(), caller.ident.id).unwrap();
    let id = caller
        .pend
        .references
        .insert(
            authorization.clone(),
            PageOffset::ZERO,
            PageCount::new(4).unwrap(),
            Access::all(),
            None,
            None,
        )
        .unwrap();
    team.space
        .views
        .claim(
            slice.identity(),
            &team.space,
            caller.ident.id,
            env::ViewControl::Caller,
            authorization,
            None,
        )
        .unwrap();
    team.space.views.commit(slice.identity(), slice.clone());

    let token = create_pole(&caller, reference_of(id), true).unwrap();
    let snapshot = pend::locate(&caller, token).unwrap();
    let meta = snapshot.mail().pole().unwrap();

    // Open 必须给整份来源：4 页、PA 次序对得上，不能拿那 2 页 slice 顶替。
    let (address, size) = pole::open(&meta, token, &team.space, rw).unwrap();
    assert_eq!(size, 4 * PAGE_SIZE);
    assert!(
        team.space
            .validate_access(VirtAddr::wrap(address), 4 * PAGE_SIZE, Access::READ)
    );
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            team.space
                .nest
                .physical_at(VirtAddr::wrap(address + index * PAGE_SIZE))
                .unwrap()
                .as_usize(),
            frame.physical_address(),
            "source page {index}"
        );
    }
    // slice 不是被复用的那一条，但它仍归这份来源：撤销时一并关掉。
    assert!(team.space.nest.translate(slice.initial_start()).is_some());
    pend::release(&caller, token).unwrap();
    assert!(team.space.nest.translate(VirtAddr::wrap(address)).is_none());
    assert!(team.space.nest.translate(slice.initial_start()).is_none());
}

pub fn lifecycle() {
    let before = TABLE_LIVE.load(Ordering::Relaxed);
    let space = SpaceBuilder::user().build().unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let peer = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .spawn()
        .unwrap();
    let recipient = peer.task().hold().unwrap();
    let rw = Access::READ | Access::WRITE;
    let reservation =
        super::support::reserve_low(&team.space, RegionId::Normal, PAGE_SIZE).unwrap();
    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    let input = super::support::attach_frames(
        &team.space,
        reservation,
        vec![frame],
        rw | Access::EXECUTE,
        rw,
    )
    .unwrap();
    // 新 ABI：Pole 从一份**来源引用**创建（完整范围 + 一次性资格都在它身上）。
    let reference = super::support::source_reference(&caller, &team.space, &input);
    let start = input.initial_start();
    let physical = team.space.nest.physical_at(start).unwrap();
    let original = input.active().unwrap()[0].current.bits();
    let kernel = crate::core::work::unit::team::kernel()
        .unwrap()
        .space
        .clone();
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let kernel_tables = kernel.nest.table_count();
    let frames = held_pages();
    let kernel_views = kernel.nest.view_count();
    for stage in 1..=5 {
        fail_pole_at(stage);
        assert_eq!(
            create_pole(&caller, reference, true),
            Err(env::PieFail::OoM)
        );
        assert!(caller.pend.pies.lock().is_empty());
        // 失败的创建不留占：输入映射仍是调用方那一段，占用、帧账与内核 VA 都不涨。
        assert!(Arc::ptr_eq(&team.space.nest.probe(start).unwrap(), &input));
        assert_eq!(input.active().unwrap()[0].current.bits(), original);
        // 资格回退仍可用（同源可重试）。
        let authorization = caller
            .pend
            .references
            .get(reference.get())
            .unwrap()
            .source()
            .clone();
        assert!(authorization.claim().is_ok());
        authorization.release_claim();
        assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
        assert_eq!(held_pages(), frames);
        assert_eq!(kernel.nest.view_count(), kernel_views);
    }
    // 保留 RAM 不占额外 kernel Resource VA：成功创建前后内核 Nest 的关系与表页都不变。
    let token = create_pole(&caller, reference, true).unwrap();
    assert_eq!(kernel.nest.view_count(), kernel_views);
    assert_eq!(kernel.nest.table_count(), kernel_tables);
    assert_eq!(held_pages(), frames);
    assert_eq!(
        create_pole(&caller, reference, true),
        Err(env::PieFail::Denied)
    );
    let snapshot = pend::locate(&caller, token).unwrap();
    let meta = snapshot.mail().pole().unwrap();
    let first = pole::open(&meta, token, &team.space, rw).unwrap();
    assert_eq!(pole::open(&meta, token, &team.space, rw).unwrap(), first);
    assert_eq!(first.0, start.as_usize());
    assert!(meta.opened(&input));
    // 同一个 token 在另一个队伍里再开一段：撤销要能把两处都关掉。
    let in_peer = pole::open(&meta, token, &peer.space, rw).unwrap();
    let peer_view = peer.space.nest.probe(VirtAddr::wrap(in_peer.0)).unwrap();
    assert!(meta.opened(&peer_view));
    drop(peer_view);
    // 关闭的同步失败：关系与关闭责任都留在 Pole，占用禁复用，同身份重试才收尾。
    identity::fail_next_shootdown(team.space.nest.asid());
    assert_eq!(pole::shut(&meta, token), Err(env::PieFail::Busy));
    assert!(team.space.nest.has_pending_invalidation());
    assert_eq!(
        super::support::reserve(&team.space, start, PAGE_SIZE).err(),
        Some(NestError::NoRegion)
    );
    assert!(weak.upgrade().is_some());
    team.space.nest.retry_invalidation().unwrap();
    pole::shut(&meta, token).unwrap();
    assert!(team.space.nest.translate(start).is_none());
    assert!(
        peer.space
            .nest
            .translate(VirtAddr::wrap(in_peer.0))
            .is_none()
    );
    // 全部 View 关掉之后来源仍在：同一授权还能重新挂载，并交付同一物理页。
    let (reopened, _) = pole::open(&meta, token, &team.space, rw).unwrap();
    assert!(
        team.space
            .validate_access(VirtAddr::wrap(reopened), PAGE_SIZE, Access::READ)
    );
    assert_eq!(
        team.space
            .nest
            .physical_at(VirtAddr::wrap(reopened))
            .unwrap(),
        physical
    );
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
    let (shared_local, _) = pole::open(&meta, child, &team.space, rw).unwrap();
    let peer_map = peer.space.nest.probe(VirtAddr::wrap(shared)).unwrap();
    assert!(meta.opened(&peer_map));
    assert_eq!(
        peer.space.nest.translate(VirtAddr::wrap(shared)).unwrap().0,
        physical
    );
    assert_eq!(
        create_pole(&recipient, reference, true),
        Err(env::PieFail::Denied)
    );
    pend::reduce(&caller, token, Permission::FETCH | Permission::VEST).unwrap();
    assert!(!snapshot.permission().contains(Permission::STORE));
    assert!(
        !team
            .space
            .nest
            .translate(VirtAddr::wrap(reopened))
            .unwrap()
            .1
            .contains(PteFlags::W)
    );
    assert!(!meta.readonly());
    assert!(!meta.exclusive());
    pend::revoke(&caller, &Arc::downgrade(&recipient), child).unwrap();
    assert!(meta.readonly());
    assert!(meta.exclusive());
    assert!(
        team.space
            .nest
            .translate(VirtAddr::wrap(shared_local))
            .is_none()
    );
    assert!(peer.space.nest.translate(VirtAddr::wrap(shared)).is_none());
    pole::shut(&meta, token).unwrap();
    assert!(weak.upgrade().is_some());
    assert!(
        team.space
            .nest
            .translate(VirtAddr::wrap(reopened))
            .is_none()
    );
    let (again, _) = pole::open(&meta, token, &team.space, Access::READ).unwrap();
    assert!(
        team.space
            .validate_access(VirtAddr::wrap(again), PAGE_SIZE, Access::READ)
    );
    assert!(
        !team
            .space
            .validate_access(VirtAddr::wrap(again), PAGE_SIZE, Access::WRITE)
    );
    assert_eq!(
        create_pole(&caller, reference, true),
        Err(env::PieFail::Denied)
    );
    let (again_peer, _) = pole::open(&meta, token, &peer.space, Access::READ).unwrap();
    pend::release(&caller, token).unwrap();
    assert!(!snapshot.alive());
    assert!(team.space.nest.translate(VirtAddr::wrap(again)).is_none());
    assert!(
        peer.space
            .nest
            .translate(VirtAddr::wrap(again_peer))
            .is_none()
    );
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
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), before);
}

/// 设备区域：借用凭证代替存储保留，不清零、不按 RAM 回收、不占内核 VA。
pub fn region() {
    let space = SpaceBuilder::user().build().unwrap();
    space
        .nest
        .attach_region(
            RegionId::Normal,
            0x4000_0000,
            crate::machine::mode::upper().as_usize(),
        )
        .unwrap();
    let team = TeamBuilder::new(space).spawn().unwrap();
    let caller = team.task().hold().unwrap();
    let kernel = crate::core::work::unit::team::kernel()
        .unwrap()
        .space
        .clone();
    let kernel_views = kernel.nest.view_count();
    let kernel_tables = kernel.nest.table_count();
    let tables = TABLE_LIVE.load(Ordering::Relaxed);

    // 借来一页真实 RAM 当设备区间：先写模式，再看它经来源与 CPU 访问后是否原样。
    let frame = Frame::allocate().unwrap();
    let weak = Arc::downgrade(&frame);
    let physical = frame.physical_address();
    let pattern = vec![0xA5u8; PAGE_SIZE];
    unsafe {
        ::core::ptr::copy_nonoverlapping(pattern.as_ptr(), frame.as_mut_ptr(), PAGE_SIZE);
    }
    let held = held_pages();

    let meta = pole::region(physical, PAGE_SIZE, caller.ident.id).unwrap();
    // 接入设备区间不占内核 VA、不动表页、也不从 RAM 取页。
    assert_eq!(kernel.nest.view_count(), kernel_views);
    assert_eq!(kernel.nest.table_count(), kernel_tables);
    assert_eq!(
        held_pages(),
        held,
        "flat: a borrowed device region takes no RAM frame"
    );

    let pie = pend::Pie::try_new(
        Mail::Pole(meta.clone()),
        env::Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .unwrap();
    let token = pie.token();
    pend::insert(&caller, pie).unwrap();
    // 借用来源不算自有 RAM：旧共享授权看的正是这个判据。
    assert!(!meta.owned());

    // CPU 访问显式建立普通 View：PA 就是借来的那一页，内容没有被清零。
    let (at, _) = pole::open(&meta, token, &team.space, Access::READ).unwrap();
    assert_eq!(
        team.space
            .nest
            .physical_at(VirtAddr::wrap(at))
            .unwrap()
            .as_usize(),
        physical
    );
    let mut seen = vec![0u8; PAGE_SIZE];
    assert!(team.space.copy_in(&mut seen, VirtAddr::wrap(at)));
    assert!(seen.iter().all(|byte| *byte == 0xA5));

    // 关闭 View 之后来源仍在：借来的存储仍归调用方，没被当 RAM 回收。
    pole::shut(&meta, token).unwrap();
    assert!(weak.upgrade().is_some());
    assert_eq!(
        unsafe { ::core::ptr::read_volatile(physical as *const u8) },
        0xA5,
        "flat: borrowed storage is not reclaimed as RAM"
    );
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);

    drop(meta);
    drop(frame);
    assert!(weak.upgrade().is_none());
    assert!(team.release_held(&caller));
    drop(caller);
    drop(team);
    drop(kernel);
}

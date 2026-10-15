#![cfg(debug_assertions)]

use alloc::sync::Arc;
use env::{ControlCall, DebugCall, EnvCall, MemoryCall, ProgramKind, RoomCall, UnitCall, VirtAddr};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageOffset, VirtAddr as KVirt};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, NestError, Placement, RegionId, View};
use crate::core::work::mail::{Mail, hole, pole};
use crate::core::work::pend::{self, Need, Permission};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::space::SpaceBuilder;

use crate::core::work::unit::task::Task;
use crate::core::work::unit::team::{Team, TeamBuilder};
use crate::entry::call as envcall;
use crate::entry::report::backtrace::StackReader;
use crate::machine::mode;
use crate::machine::switcher::context::{Gprs, TrapContext};

struct Caller {
    team: Arc<Team>,
    task: Arc<Task>,
    /// 描述用的可写页：**建 Caller 时就备齐**，之后所有读数基线都包含它。
    scratch: core::cell::RefCell<Option<Arc<View>>>,
    /// 地址 → （初始起点, 跨度, 关系身份）：旧调用面按 VA 归还/改权，
    /// 这里换成准确身份与原坐标。
    mapped: core::cell::RefCell<alloc::vec::Vec<(usize, usize, usize)>>,
}

/// 夹具自己的一块可写页（描述区）。地址取窗口里一个独立位置，
/// 不占被测 Lowest 选址的预期首址；建立后当场确认可写。
const SCRATCH_AT: usize = 0x7000_0000;

fn scratch_view(space: &Space) -> Result<Arc<View>, NestError> {
    use crate::core::memory::address::PageIndex;
    use crate::core::memory::flat::Flat;
    // 真实的一页帧：新逻辑来源只有物化过才有已发布 PTE，
    // `Space::copy_out` 不会替夹具模拟用户缺页。
    let frame = crate::core::memory::flat::Frame::allocate()?;
    let count = PageCount::new(1).ok_or(NestError::NotAligned)?;
    let source = Flat::from_frames(count, &[(PageIndex::new(0), frame)])?.whole();
    space.nest.retry_invalidation()?;
    let reservation = space.nest.reserve(Placement::exact(
        RegionId::Normal,
        KVirt::wrap(SCRATCH_AT),
        PAGE_SIZE,
    )?)?;
    let view = space.nest.attach(
        reservation,
        source,
        Access::all(),
        Access::READ | Access::WRITE,
    )?;
    assert!(
        space.validate_access(KVirt::wrap(SCRATCH_AT), PAGE_SIZE, Access::WRITE),
        "scratch page must be writable before baselines"
    );
    Ok(view)
}

impl Caller {
    fn new(supervisor: bool) -> Self {
        let space = if supervisor {
            SpaceBuilder::supervisor()
        } else {
            SpaceBuilder::user()
        }
        .build()
        .unwrap();
        space
            .nest
            .attach_region(RegionId::Normal, 0x4000_0000, mode::upper().as_usize())
            .unwrap();
        let team = TeamBuilder::new(space).spawn().unwrap();
        let task = team.task().hold().unwrap();
        let caller = Self {
            team,
            task,
            scratch: core::cell::RefCell::new(None),
            mapped: core::cell::RefCell::new(alloc::vec::Vec::new()),
        };
        let view = scratch_view(&caller.team.space).expect("scratch page");
        *caller.scratch.borrow_mut() = Some(view);
        caller
    }

    fn raw(&self, slot: usize, regs: [usize; 6]) -> isize {
        let frame = unsafe { &mut *(self.task.ident.frame_pa.as_usize() as *mut TrapContext) };
        for (i, value) in regs.into_iter().enumerate() {
            frame.gpr.set_x(Gprs::A0 + i, value);
        }
        frame.gpr.set_x(Gprs::A7, slot);
        assert!(envcall::dispatch(frame, self.task.ident.clone()).is_some());
        frame.gpr.x(Gprs::A0) as isize
    }

    fn call(&self, call: EnvCall) -> isize {
        let (slot, regs) = match call {
            EnvCall::Memory(call) => (call.slot(), call.pack()),
            EnvCall::Unit(call) => (call.slot(), call.pack()),
            EnvCall::Room(call) => (call.slot(), call.pack()),
            EnvCall::Debug(call) => (call.slot(), call.pack()),
            EnvCall::Control(call) => (call.slot(), call.pack()),
            _ => unreachable!(),
        };
        self.raw(slot, regs)
    }

    /// 双值返回那一格（`Attach`）：读 `a0` / `a1`。
    fn pair(&self, call: EnvCall) -> (isize, usize) {
        let (slot, regs) = match call {
            EnvCall::Memory(call) => (call.slot(), call.pack()),
            _ => unreachable!(),
        };
        let frame = unsafe { &mut *(self.task.ident.frame_pa.as_usize() as *mut TrapContext) };
        for (i, value) in regs.into_iter().enumerate() {
            frame.gpr.set_x(Gprs::A0 + i, value);
        }
        frame.gpr.set_x(Gprs::A7, slot);
        assert!(envcall::dispatch(frame, self.task.ident.clone()).is_some());
        (frame.gpr.x(Gprs::A0) as isize, frame.gpr.x(Gprs::A1))
    }

    fn allocate(&self, size: usize) -> usize {
        self.attach_alloc(size, 6).1
    }

    /// 描述区地址（建 Caller 时已备齐）。
    fn scratch_page(&self) -> usize {
        self.scratch
            .borrow()
            .as_ref()
            .expect("scratch page")
            .initial_start()
            .as_usize()
    }

    /// 旧 `Mmap` 的等价：`pole=NONE` 取惰性匿名来源，否则取该 Pole 的授权窗口。
    /// 成功时像旧调用一样回**地址**，并把地址→身份记下供归还用。
    fn map(
        &self,
        team: env::TeamId,
        at: usize,
        size: usize,
        pole: env::PieToken,
        offset: usize,
        flags: u64,
    ) -> isize {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) || !offset.is_multiple_of(PAGE_SIZE) {
            return env::MemoryFail::NotAligned.code();
        }
        let count = size / PAGE_SIZE;
        let source = match pole {
            env::PieToken::NONE => env::AcquireSource::Allocation(
                env::Allocation::new(size, PAGE_SIZE).expect("allocation"),
            ),
            token => env::AcquireSource::Source {
                token,
                offset: offset / PAGE_SIZE,
                count,
            },
        };
        let handle = self.call(EnvCall::Memory(MemoryCall::Acquire {
            source,
            retention: env::PieToken::NONE,
        }));
        if handle <= 0 {
            return handle;
        }
        let scratch = self.scratch_page();
        self.write_words(scratch, &[handle as usize, 0, count]);
        self.write_words(scratch + 32, &[if at == 0 { 0 } else { 2 }, at, PAGE_SIZE]);
        let (view, address) = self.pair(EnvCall::Memory(MemoryCall::Attach {
            range: VirtAddr::new(scratch),
            team,
            placement: VirtAddr::new(scratch + 32),
            access: flags,
            control: env::ViewControl::Caller,
        }));
        assert_eq!(
            self.call(EnvCall::Memory(MemoryCall::Release {
                reference: reference(handle as usize),
            })),
            0
        );
        if view <= 0 {
            return view;
        }
        self.mapped
            .borrow_mut()
            .push((address as usize, size, view as usize));
        address as isize
    }

    /// 某地址下面那段关系（原 `ViewId` + 初始起点与跨度）。
    fn identity_at(&self, addr: usize) -> Option<(usize, usize, usize)> {
        self.mapped
            .borrow()
            .iter()
            .rev()
            .find(|(start, span, _)| addr >= *start && addr < start + span)
            .copied()
    }

    /// 把旧 `(addr, size)` 折成该关系上的选择。
    ///
    /// 部分撤销/改权必须用 `Pages` 且以**原 View 坐标**为准：
    /// 只有请求恰好等于整段才是 `Whole`；越界/不对齐按旧口径拒。
    fn select_at(
        &self,
        addr: usize,
        mut size: usize,
    ) -> Result<(usize, env::ViewSelection), isize> {
        let Some((start, span, view)) = self.identity_at(addr) else {
            return Err(env::MemoryFail::Denied.code());
        };
        if size == 0 {
            // 旧 basic 口径：零字节请求就是“这一段”。
            size = span - (addr - start);
        }
        if !addr.is_multiple_of(PAGE_SIZE)
            || size == 0
            || !size.is_multiple_of(PAGE_SIZE)
            || addr.checked_add(size).is_none_or(|end| end > start + span)
        {
            return Err(env::MemoryFail::NoRegion.code());
        }
        let selection = if addr == start && size == span {
            env::ViewSelection::Whole
        } else {
            match env::ViewSelection::pages((addr - start) / PAGE_SIZE, size / PAGE_SIZE) {
                Some(selection) => selection,
                None => return Err(env::MemoryFail::NoRegion.code()),
            }
        };
        Ok((view, selection))
    }

    /// 旧 `Deallocate` / `Munmap` 的等价：按原坐标 Detach。
    fn detach_at(&self, addr: usize, size: usize) -> isize {
        let (view, selection) = match self.select_at(addr, size) {
            Ok(selected) => selected,
            Err(code) => return code,
        };
        let view = env::ViewId::from_bytes(&view.to_le_bytes()).expect("recorded view");
        self.call(EnvCall::Memory(MemoryCall::Detach {
            team: env::TeamId::new(0),
            view,
            selection,
        }))
    }

    /// 旧 `Mprotect` 的等价：按原坐标 SetAccess（`flags` 仍是线上的 R/W/X 位）。
    fn set_access_at(&self, addr: usize, size: usize, flags: u64) -> isize {
        let (view, selection) = match self.select_at(addr, size) {
            Ok(selected) => selected,
            Err(code) => return code,
        };
        let view = env::ViewId::from_bytes(&view.to_le_bytes()).expect("recorded view");
        self.call(EnvCall::Memory(MemoryCall::SetAccess {
            team: env::TeamId::new(0),
            view,
            selection,
            access: flags,
        }))
    }

    fn write_words(&self, at: usize, words: &[usize]) {
        let mut bytes = [0u8; 32];
        for (index, word) in words.iter().enumerate() {
            bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_ne_bytes());
        }
        assert!(
            self.team
                .space
                .copy_out(&bytes[..words.len() * 8], KVirt::wrap(at)),
            "descriptor at {at:#x}"
        );
    }

    /// 新 ABI 的便利分配：Acquire → Attach → Release，返回 `(ViewId, 地址)`。
    fn attach_alloc(&self, size: usize, flags: u64) -> (usize, usize) {
        let (view, at, reference) = self.attach_alloc_reference(size, flags);
        assert_eq!(
            self.call(EnvCall::Memory(MemoryCall::Release { reference })),
            0
        );
        (view, at)
    }

    fn attach_alloc_reference(
        &self,
        size: usize,
        flags: u64,
    ) -> (usize, usize, env::FlatReference) {
        let bytes = size.max(1).next_multiple_of(PAGE_SIZE);
        let count = bytes / PAGE_SIZE;
        let handle = self.call(EnvCall::Memory(MemoryCall::Acquire {
            source: env::AcquireSource::Allocation(
                env::Allocation::new(bytes, PAGE_SIZE).expect("allocation"),
            ),
            retention: env::PieToken::NONE,
        }));
        assert!(handle > 0, "acquire: {handle}");
        // 旧 `Allocate` 是 eager：先把来源物化齐再挂。夹具沿用同一前置，
        // 否则拿回来的关系没有已发布 PTE（新 ABI 的缺页不在这里模拟）。
        self.task
            .pend
            .references
            .get(handle as usize)
            .expect("hold")
            .source()
            .own_range()
            .expect("private range")
            .materialize()
            .expect("materialize");
        let scratch = self.scratch_page();
        self.write_words(scratch, &[handle as usize, 0, count]);
        self.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
        let (view, at) = self.pair(EnvCall::Memory(MemoryCall::Attach {
            range: VirtAddr::new(scratch),
            team: env::TeamId::new(0),
            placement: VirtAddr::new(scratch + 32),
            access: flags,
            control: env::ViewControl::Caller,
        }));
        assert!(view > 0, "attach: {view}");
        // 旧调用面按 VA 归还/改权：把这次分配的地址与身份记下。
        self.mapped.borrow_mut().push((at, bytes, view as usize));
        (view as usize, at, reference(handle as usize))
    }
}

impl Drop for Caller {
    fn drop(&mut self) {
        // 夹具自己的描述区也回收：净零读数里不含这块测试资源。
        if let Some(view) = self.scratch.borrow_mut().take()
            && let Some(nest) = view.owner()
        {
            let _ = nest.dispose(view);
        }
        self.team.release_held(&self.task);
        self.team.prune_tasks();
    }
}

pub fn abi_and_privilege() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    use env::wire::{Decode, Wire};
    let user = Caller::new(false);
    assert!(user.raw(3usize << 32, [0; 6]) < 0);
    assert!(user.raw((2usize << 32) | 99, [0; 6]) < 0);
    assert!(user.raw((1usize << 32) | 5, [2, 0, 0, 0, 0, 0]) < 0);
    assert!(user.raw((5usize << 32) | 2, [0, 3, 0, 0, 0, 0]) < 0);
    assert!(user.raw((7usize << 32) | 4, [0, 0, usize::MAX, 0, 0, 0]) < 0);
    assert!(matches!(
        bool::unpack(&[2, 0, 0, 0, 0, 0], &mut 0),
        Err(Decode::Invalid)
    ));
    let build = |kind| {
        EnvCall::Unit(UnitCall::Build {
            kind,
            sire: user.task.ident.id,
        })
    };
    let heirs = user.task.heir_count();
    assert_eq!(
        user.call(build(ProgramKind::Supervisor)),
        env::UnitFail::Denied.code()
    );
    assert_eq!(user.task.heir_count(), heirs);
    assert_eq!(
        user.call(build(ProgramKind::User)),
        env::UnitFail::Denied.code()
    );
    let ordinary = crate::core::work::mail::nole::NoleMeta::new(user.task.ident.id);
    user.task.pend.pies.lock().push(
        pend::Pie::try_new(
            Mail::Nole(ordinary),
            env::Mark::NONE,
            Permission::FETCH | Permission::VEST,
            None,
        )
        .expect("pie allocation"),
    );
    assert_eq!(
        user.call(build(ProgramKind::User)),
        env::UnitFail::Denied.code()
    );
    assert_eq!(
        user.call(EnvCall::Room(RoomCall::Doom {
            task: user.task.ident.id
        })),
        env::RoomFail::Denied.code()
    );
    assert!(user.task.held());
    let mut registry = crate::entry::boot::resources::Registry::default();
    crate::entry::call::resources::register(&mut registry).unwrap();
    registry.freeze().unwrap().grant(&user.task).unwrap();
    assert_eq!(
        user.call(build(ProgramKind::Supervisor)),
        env::UnitFail::Denied.code()
    );
    assert!(user.call(build(ProgramKind::User)) > 0);
    assert_eq!(
        user.call(EnvCall::Room(RoomCall::Doom {
            task: env::TaskId::new(usize::MAX),
        })),
        env::RoomFail::Dead.code()
    );
    let supervisor = Caller::new(true);
    assert_eq!(
        supervisor.call(EnvCall::Room(RoomCall::Doom {
            task: user.task.ident.id,
        })),
        env::RoomFail::Denied.code()
    );
    assert_eq!(
        supervisor.call(build(ProgramKind::User)),
        env::UnitFail::Denied.code()
    );
    let mut registry = crate::entry::boot::resources::Registry::default();
    crate::entry::call::resources::register(&mut registry).unwrap();
    registry.freeze().unwrap().grant(&supervisor.task).unwrap();
    for kind in [ProgramKind::User, ProgramKind::Supervisor] {
        assert!(
            supervisor.call(EnvCall::Unit(UnitCall::Build {
                kind,
                sire: supervisor.task.ident.id
            })) > 0
        );
    }
    assert_eq!(
        supervisor.call(build(ProgramKind::User)),
        env::UnitFail::Denied.code()
    );
    let space = SpaceBuilder::user().build().unwrap();
    space
        .nest
        .attach_region(RegionId::Normal, 0x4000_0000, mode::upper().as_usize())
        .unwrap();
    let child = TeamBuilder::new(space)
        .sire(crate::core::work::unit::task::TaskWeak::stored(
            Arc::downgrade(&supervisor.task),
            crate::core::work::unit::copies::Origin::Sire,
        ))
        .spawn()
        .unwrap();
    let parent = child.task().hold().unwrap();
    let delegated = supervisor.call(EnvCall::Unit(UnitCall::Build {
        kind: ProgramKind::User,
        sire: parent.ident.id,
    }));
    assert!(delegated > 0);
    let delegated = env::TeamId::new(delegated as usize);
    assert!(supervisor.task.heir(delegated).is_none());
    assert!(parent.heir(delegated).is_some());
    assert!(supervisor.task.clan(delegated).is_some());
    assert_eq!(
        supervisor.call(EnvCall::Unit(UnitCall::Oust { team: delegated })),
        0
    );
    assert!(parent.heir(delegated).is_none());
    let peer = supervisor.team.task().hold().unwrap();
    let sibling = supervisor.call(EnvCall::Unit(UnitCall::Build {
        kind: ProgramKind::User,
        sire: peer.ident.id,
    }));
    assert!(sibling > 0);
    let sibling = env::TeamId::new(sibling as usize);
    assert!(peer.heir(sibling).is_some());
    assert!(supervisor.task.clan(sibling).is_some());
    assert_eq!(
        supervisor.call(EnvCall::Unit(UnitCall::Oust { team: sibling })),
        0
    );
    assert_eq!(
        supervisor.call(EnvCall::Room(RoomCall::Doom {
            task: env::TaskId::new(usize::MAX)
        })),
        env::RoomFail::Dead.code()
    );
}

pub fn memory() {
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use crate::core::memory::nest::table::TABLE_LIVE;
    use core::sync::atomic::Ordering;

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let zero = user.allocate(0);
    assert_eq!(user.detach_at(zero, 0), 0);
    let frames = frame_occupied() - block_occupied();
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let page = user.allocate(PAGE_SIZE);
    let page_id = user.identity_at(page).unwrap().2;
    assert_eq!(user.detach_at(page, PAGE_SIZE), 0);
    assert_eq!(frame_occupied() - block_occupied(), frames);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(
        user.detach_at(page, PAGE_SIZE),
        env::MemoryFail::Denied.code()
    );
    let at = user.allocate(2 * PAGE_SIZE);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: env::TeamId::new(0),
            view: view_handle(page_id),
            selection: env::ViewSelection::Whole,
        })),
        env::MemoryFail::Denied.code(),
        "a retired id must not detach the replacement at the same VA"
    );
    let before = user
        .team
        .space
        .nest
        .translate(KVirt::wrap(at))
        .unwrap()
        .1
        .bits();
    for flags in [1, 16, 32, 64, 128, 256, 1u64 << 63, 0, 4] {
        assert!(user.set_access_at(at, PAGE_SIZE, flags) < 0);
        assert_eq!(
            user.team
                .space
                .nest
                .translate(KVirt::wrap(at))
                .unwrap()
                .1
                .bits(),
            before
        );
    }
    for flags in [2, 8, 10, 6] {
        assert_eq!(user.set_access_at(at, PAGE_SIZE, flags), 0);
        let actual = user
            .team
            .space
            .nest
            .translate(KVirt::wrap(at))
            .unwrap()
            .1
            .bits();
        assert_eq!(actual & !14, before & !14);
        assert_eq!(actual & 14, flags);
    }
    for internal in [
        user.task.ident.frame_va.as_usize(),
        crate::machine::layout::TRAMPOLINE.as_usize(),
        mode::lower().as_usize(),
        at + (mode::upper().as_usize() << 1),
    ] {
        assert!(user.raw((2usize << 32) | 8, [0, internal, 0, 0, 0, 0]) < 0);
        assert!(user.raw((2usize << 32) | 9, [0, internal, 0, 0, 0, 6]) < 0);
        let handle = user.call(EnvCall::Memory(MemoryCall::Acquire {
            source: env::AcquireSource::Allocation(
                env::Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap(),
            ),
            retention: env::PieToken::NONE,
        }));
        assert!(handle > 0);
        let scratch = user.scratch_page();
        user.write_words(scratch, &[handle as usize, 0, 1]);
        user.write_words(scratch + 32, &[2, internal, PAGE_SIZE]);
        assert!(
            user.pair(EnvCall::Memory(MemoryCall::Attach {
                team: env::TeamId::new(0),
                range: VirtAddr::new(scratch),
                placement: VirtAddr::new(scratch + 32),
                access: 6,
                control: env::ViewControl::Caller,
            }))
            .0 < 0
        );
        assert_eq!(
            user.call(EnvCall::Memory(MemoryCall::Release {
                reference: reference(handle as usize),
            })),
            0
        );
    }
    for slot in 0..5 {
        assert_eq!(
            user.raw((2usize << 32) | slot, [0; 6]),
            env::DispatchFail::Unknown.code()
        );
    }
    for size in [usize::MAX, usize::MAX - PAGE_SIZE + 1] {
        for (slot, words) in [
            (5, [0, size, PAGE_SIZE, 0, 0, 0]),
            (5, [1, 1, 0, size, 0, 0]),
            (8, [0, 1, 1, 0, size, 0]),
            (9, [0, 1, 1, 0, size, 6]),
        ] {
            let frames_before = frame_occupied() - block_occupied();
            let result = user.raw((2usize << 32) | slot, words);
            if slot == 5 && words[0] == 0 && env::Allocation::new(size, PAGE_SIZE).is_some() {
                assert!(result > 0, "a checked logical source needs no eager frames");
                assert_eq!(
                    user.call(EnvCall::Memory(MemoryCall::Release {
                        reference: reference(result as usize),
                    })),
                    0
                );
            } else {
                assert_eq!(result, env::DispatchFail::Unknown.code());
            }
            assert_eq!(frame_occupied() - block_occupied(), frames_before);
        }
        assert!(
            user.call(EnvCall::Unit(UnitCall::Spawn {
                team: env::TeamId::new(0),
                entry: 0,
                args: VirtAddr::new(0),
                count: 0,
                stack: size
            })) < 0
        );
    }
    assert_eq!(user.detach_at(at, 2 * PAGE_SIZE), 0);
    assert!(user.team.space.nest.translate(KVirt::wrap(at)).is_none());

    let fixed = 0x4004_0000;
    assert_eq!(
        user.map(
            env::TeamId::new(0),
            fixed,
            3 * PAGE_SIZE,
            env::PieToken::NONE,
            0,
            6
        ),
        fixed as isize
    );
    assert_eq!(user.set_access_at(fixed + PAGE_SIZE, PAGE_SIZE, 2), 0);
    for (page, writable) in [
        (fixed, true),
        (fixed + PAGE_SIZE, false),
        (fixed + 2 * PAGE_SIZE, true),
    ] {
        let map = user.team.space.nest.probe(KVirt::wrap(page)).unwrap();
        user.team
            .space
            .nest
            .materialize_active(
                map.range(
                    PageOffset::new((page - map.initial_start().as_usize()) / PAGE_SIZE),
                    PageCount::new(1).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            user.team
                .space
                .nest
                .translate(KVirt::wrap(page))
                .unwrap()
                .1
                .contains(PteFlags::W),
            writable
        );
    }
    assert_eq!(user.detach_at(fixed + PAGE_SIZE, PAGE_SIZE), 0);
    assert_eq!(user.detach_at(fixed, 3 * PAGE_SIZE), 0);
    assert!(user.team.space.nest.translate(KVirt::wrap(fixed)).is_none());
}

pub fn pointers() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let at = user.allocate(PAGE_SIZE);
    assert!(user.team.space.copy_out(b"readable", KVirt::wrap(at)));
    let mut bytes = [0xa5; 8];
    for (va, len) in [
        (0, 8),
        (user.task.ident.frame_va.as_usize(), 8),
        (crate::machine::layout::TRAMPOLINE.as_usize(), 8),
        (at + PAGE_SIZE - 4, 8),
        (at, usize::MAX),
        (usize::MAX - 3, 8),
    ] {
        assert!(
            !user
                .team
                .space
                .validate_access(KVirt::wrap(va), len, Access::READ)
        );
        assert!(
            !user
                .team
                .space
                .validate_access(KVirt::wrap(va), len, Access::WRITE)
        );
        assert!(
            user.call(EnvCall::Debug(DebugCall::Put {
                buf: VirtAddr::new(va),
                len
            })) < 0
        );
        if !user
            .team
            .space
            .validate_access(KVirt::wrap(va), size_of::<usize>(), Access::READ)
        {
            assert!(
                user.call(EnvCall::Unit(UnitCall::Spawn {
                    team: env::TeamId::new(0),
                    entry: 0,
                    args: VirtAddr::new(va),
                    count: 1,
                    stack: 0
                })) < 0
            );
        }
    }
    assert!(
        !user
            .team
            .space
            .copy_in(&mut bytes, KVirt::wrap(at + PAGE_SIZE - 4))
    );
    assert_eq!(bytes, [0xa5; 8]);
    assert!(user.team.space.copy_in(&mut [], KVirt::wrap(0)));
    assert!(user.team.space.copy_out(&[], KVirt::wrap(0)));
    assert_eq!(
        user.call(EnvCall::Debug(DebugCall::Put {
            buf: VirtAddr::new(0),
            len: 0
        })),
        env::DebugFail::Denied.code()
    );
    assert!(
        user.call(EnvCall::Debug(DebugCall::Get {
            buf: VirtAddr::new(user.task.ident.frame_va.as_usize()),
            len: 8
        })) < 0
    );
    assert!(
        user.call(EnvCall::Control(ControlCall::Backtrace {
            buf: user.task.ident.frame_va.as_usize(),
            frames: 1
        })) < 0
    );
    assert!(
        user.call(EnvCall::Control(ControlCall::Backtrace {
            buf: at,
            frames: usize::MAX
        })) < 0
    );
    let mut reader = StackReader::user(user.team.space.clone());
    assert!(reader.word(user.task.ident.frame_va.as_usize()).is_none());
    assert_eq!(user.set_access_at(at, PAGE_SIZE, 8), 0);
    assert!(!user.team.space.copy_in(&mut bytes, KVirt::wrap(at)));
    assert!(
        user.call(EnvCall::Debug(DebugCall::Put {
            buf: VirtAddr::new(at),
            len: 8
        })) < 0
    );
    assert!(user.team.space.instruction_byte(KVirt::wrap(at)).is_some());
    let foreign = Caller::new(false);
    assert!(
        !foreign
            .team
            .space
            .validate_access(KVirt::wrap(at), 8, Access::READ)
    );
}

/// 由线上那一格取一枚引用句柄（外部测试只有解码这一扇门）。
fn reference(raw: usize) -> env::FlatReference {
    match MemoryCall::from_wire((2usize << 32) | 6, &[raw, 0, 0, 0, 0, 0]).unwrap() {
        MemoryCall::Release { reference } => reference,
        other => panic!("unexpected release: {other:?}"),
    }
}

/// 新五操作里的 `Acquire` / `Release`：来源持有、clone、释放与资格寿命。
pub fn lifecycle() {
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use env::{AcquireSource, Allocation, MemoryFail, PieToken, ReferenceRange};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    let acquire =
        |source, retention| user.call(EnvCall::Memory(MemoryCall::Acquire { source, retention }));
    let release = |raw: usize| {
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(raw),
        }))
    };

    // 新 Allocation：一份来源持有 + 一份未消费的创建资格（此处不物化，不占帧）。
    let frames = frame_occupied() - block_occupied();
    let first = acquire(
        AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
        PieToken::NONE,
    );
    assert!(first > 0, "acquire allocation: {first}");
    let hold = task
        .pend
        .references
        .get(first as usize)
        .expect("registered hold");
    // 测试自己持着授权记录：资格结束必须是显式语义，不靠 `Arc` 死亡。
    let auth = hold.source().clone();

    // clone 子范围：同一份来源授权，不复制资格，也不额外占帧。
    let cloned = acquire(
        AcquireSource::Reference(
            ReferenceRange::new(reference(first as usize), 0, 1).expect("sub-range"),
        ),
        PieToken::NONE,
    );
    assert!(cloned > 0 && cloned != first, "clone range: {cloned}");
    let cloned_hold = task
        .pend
        .references
        .get(cloned as usize)
        .expect("clone hold");
    assert!(
        Arc::ptr_eq(hold.source(), cloned_hold.source()),
        "a sub-range stays on one authorization record"
    );
    // 私有阶段：引用向来源授权解析得到可用范围。
    assert!(
        hold.resolve().is_some(),
        "private source resolves its range"
    );
    assert_eq!(cloned_hold.resolve().map(|r| r.page_count().get()), Some(1));
    assert_eq!(
        frame_occupied() - block_occupied(),
        frames,
        "clone adds no frame"
    );
    // 测试自己手里的弧不参与“引用是否还在”的判定：先交回。
    drop(hold);
    drop(cloned_hold);

    // Release 只解这一份：不 Detach、不动其他引用、不分配。
    assert_eq!(release(cloned as usize), 0, "release clone");
    assert!(task.pend.references.get(cloned as usize).is_none());
    assert!(
        task.pend.references.get(first as usize).is_some(),
        "other hold stays"
    );
    assert!(
        task.pend
            .references
            .get(first as usize)
            .and_then(|hold| hold.resolve())
            .is_some(),
        "the surviving reference still resolves its range"
    );
    assert!(
        auth.claim().is_ok(),
        "one release does not end the creation right"
    );
    auth.release_claim();
    assert_eq!(
        frame_occupied() - block_occupied(),
        frames,
        "release adds no frame"
    );
    assert_eq!(
        release(cloned as usize),
        MemoryFail::Denied.code(),
        "the same handle cannot be released twice"
    );
    assert_eq!(
        release(isize::MAX as usize),
        MemoryFail::Denied.code(),
        "an unknown handle is not this table's"
    );

    auth.own_range().unwrap().materialize().unwrap();
    let scratch = user.scratch_page();
    user.write_words(scratch, &[first as usize, 0, 1]);
    user.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
    let (view_id, address) = user.pair(EnvCall::Memory(MemoryCall::Attach {
        range: VirtAddr::new(scratch),
        team: env::TeamId::new(0),
        placement: VirtAddr::new(scratch + 32),
        access: 6,
        control: env::ViewControl::Caller,
    }));
    assert!(view_id > 0);
    let view = user
        .team
        .space
        .views
        .locate(view_id as usize)
        .unwrap()
        .view()
        .unwrap()
        .clone();
    assert!(user.team.space.copy_out(b"live", KVirt::wrap(address)));

    // 活 View 仍保数据；最后普通引用释放永久结束资格。
    assert_eq!(release(first as usize), 0, "release last");
    assert!(
        auth.claim().is_err(),
        "the creation right ends with the last ordinary reference"
    );
    assert!(view.is_live());
    let mut bytes = [0; 4];
    assert!(user.team.space.copy_in(&mut bytes, KVirt::wrap(address)));
    assert_eq!(&bytes, b"live");
    let data = acquire(
        AcquireSource::View {
            team: env::TeamId::new(0),
            view: view_handle(view_id as usize),
            offset: 0,
            count: 1,
        },
        PieToken::NONE,
    );
    assert!(data > 0, "the private owner can reacquire data");
    assert_eq!(
        crate::entry::call::pie::create_pole(task, reference(data as usize), true),
        Err(env::PieFail::Denied),
        "a live View cannot revive the ended creation qualification"
    );
    assert_eq!(release(data as usize), 0);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: env::TeamId::new(0),
            view: view_handle(view_id as usize),
            selection: env::ViewSelection::Whole,
        })),
        0
    );
    drop(view);
    drop(auth);
    assert_eq!(frame_occupied() - block_occupied(), frames);
    let full = acquire(
        AcquireSource::Allocation(Allocation::new(2 * PAGE_SIZE, PAGE_SIZE).unwrap()),
        PieToken::NONE,
    );
    assert!(full > 0);
    let slice = acquire(
        AcquireSource::Reference(ReferenceRange::new(reference(full as usize), 0, 1).unwrap()),
        PieToken::NONE,
    );
    assert!(slice > 0);
    assert_eq!(
        acquire(
            AcquireSource::Reference(ReferenceRange::new(reference(slice as usize), 1, 1).unwrap()),
            PieToken::NONE
        ),
        MemoryFail::NoRegion.code(),
        "a clone cannot escape its input slice"
    );
    user.write_words(scratch, &[full as usize, 0, 1]);
    user.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
    let (slice_view, _) = user.pair(EnvCall::Memory(MemoryCall::Attach {
        range: VirtAddr::new(scratch),
        team: env::TeamId::new(0),
        placement: VirtAddr::new(scratch + 32),
        access: 6,
        control: env::ViewControl::Caller,
    }));
    assert!(slice_view > 0);
    assert_eq!(
        acquire(
            AcquireSource::View {
                team: env::TeamId::new(0),
                view: view_handle(slice_view as usize),
                offset: 0,
                count: 2,
            },
            PieToken::NONE
        ),
        MemoryFail::NoRegion.code(),
        "View acquisition stays inside its source window"
    );
    let forged_retention = PieToken::from_bytes(&(isize::MAX as usize).to_le_bytes()).unwrap();
    assert_eq!(
        acquire(
            AcquireSource::View {
                team: env::TeamId::new(0),
                view: view_handle(slice_view as usize),
                offset: 0,
                count: 1,
            },
            forged_retention
        ),
        MemoryFail::Denied.code(),
        "a View input must also validate retention"
    );
    assert_eq!(release(slice as usize), 0);
    assert_eq!(release(full as usize), 0);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: env::TeamId::new(0),
            view: view_handle(slice_view as usize),
            selection: env::ViewSelection::Whole,
        })),
        0
    );
}

pub fn preparation_failures() {
    use crate::entry::call::memory::fail_prepare_at;
    use env::{
        AcquireSource, Allocation, MemoryFail, PieToken, TeamId, ViewControl, ViewSelection,
    };
    crate::core::work::room::scheduler::init().unwrap();
    let user = Caller::new(false);
    let table = &user.task.pend.references;
    fail_prepare_at(5);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
            retention: PieToken::NONE,
        })),
        MemoryFail::OoM.code()
    );
    assert_eq!(table.len(), 0);
    let (original_id, original_at, source) = user.attach_alloc_reference(PAGE_SIZE, 6);
    assert!(user.team.space.copy_out(b"retry", KVirt::wrap(original_at)));
    let physical = user
        .team
        .space
        .nest
        .physical_at(KVirt::wrap(original_at))
        .unwrap();
    let auth = table.get(source.get()).unwrap().source().clone();
    let views = user.team.space.nest.view_count();
    let scratch = user.scratch_page();
    user.write_words(scratch, &[source.get(), 0, 1]);
    user.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
    let attach = || {
        user.pair(EnvCall::Memory(MemoryCall::Attach {
            range: VirtAddr::new(scratch),
            team: TeamId::new(0),
            placement: VirtAddr::new(scratch + 32),
            access: 6,
            control: ViewControl::Caller,
        }))
    };
    fail_prepare_at(7);
    assert_eq!(attach().0, MemoryFail::OoM.code());
    assert_eq!(user.team.space.nest.view_count(), views);
    assert_eq!(
        user.team.space.nest.physical_at(KVirt::wrap(original_at)),
        Some(physical)
    );
    assert_eq!(table.len(), 1);
    auth.claim().unwrap();
    auth.release_claim();
    let (second, second_at) = attach();
    assert!(second > 0);
    let protect = || {
        user.call(EnvCall::Memory(MemoryCall::SetAccess {
            team: TeamId::new(0),
            view: view_handle(original_id),
            selection: ViewSelection::Whole,
            access: 2,
        }))
    };
    fail_prepare_at(9);
    assert_eq!(protect(), MemoryFail::OoM.code());
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(original_at), PAGE_SIZE, Access::WRITE)
    );
    assert_eq!(protect(), 0);
    let detach = |id| {
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: TeamId::new(0),
            view: view_handle(id),
            selection: ViewSelection::Whole,
        }))
    };
    fail_prepare_at(8);
    assert_eq!(detach(second as usize), MemoryFail::OoM.code());
    assert!(
        user.team
            .space
            .nest
            .translate(KVirt::wrap(second_at))
            .is_some()
    );
    assert_eq!(detach(second as usize), 0);
    let capacity = table.capacity();
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    assert_eq!(table.capacity(), capacity);
    assert_eq!(table.len(), 0);
    assert!(
        user.team
            .space
            .nest
            .translate(KVirt::wrap(original_at))
            .is_some()
    );
    let mut bytes = [0; 5];
    assert!(
        user.team
            .space
            .copy_in(&mut bytes, KVirt::wrap(original_at))
    );
    assert_eq!(&bytes, b"retry");
    assert_eq!(detach(original_id), 0);
}

pub fn branches_and_retention() {
    use env::{AcquireSource, Mark, MemoryFail, PieToken, TeamId, ViewControl};
    crate::core::work::room::scheduler::init().unwrap();
    let owner = Caller::new(false);
    let left = Caller::new(false);
    let right = Caller::new(false);
    let (_, source_at, source) = owner.attach_alloc_reference(PAGE_SIZE, 6);
    let source_frame = {
        let view = owner.team.space.nest.probe(KVirt::wrap(source_at)).unwrap();
        let frames = view.frames_in(KVirt::wrap(source_at), PAGE_SIZE).unwrap();
        Arc::downgrade(&frames[0].1)
    };
    let root = crate::entry::call::pie::create_pole(&owner.task, source, true).unwrap();
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let permission = Permission::FETCH | Permission::STORE | Permission::VEST;
    let grant = |caller: &Caller| {
        let raw = pend::accord(
            &owner.task,
            root,
            &Arc::downgrade(&caller.task),
            permission,
            Mark::NONE,
        )
        .unwrap();
        PieToken::from_bytes(&raw.to_le_bytes()).unwrap()
    };
    let left_token = grant(&left);
    let right_token = grant(&right);
    let acquire = |caller: &Caller, token, retention| {
        caller.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token,
                offset: 0,
                count: 1,
            },
            retention,
        }))
    };
    let left_ref = acquire(&left, left_token, PieToken::NONE);
    let right_ref = acquire(&right, right_token, PieToken::NONE);
    assert!(left_ref > 0 && right_ref > 0);
    let left_hold = left.task.pend.references.get(left_ref as usize).unwrap();
    let right_hold = right.task.pend.references.get(right_ref as usize).unwrap();
    assert!(Arc::ptr_eq(left_hold.source(), right_hold.source()));
    assert!(!Arc::ptr_eq(
        left_hold.grant().unwrap(),
        right_hold.grant().unwrap()
    ));
    assert_eq!(left_hold.branch().unwrap().0, left_token);
    assert_eq!(right_hold.branch().unwrap().0, right_token);
    let left_at = left.map(TeamId::new(0), 0, PAGE_SIZE, left_token, 0, 6);
    let right_at = right.map(TeamId::new(0), 0, PAGE_SIZE, right_token, 0, 6);
    assert!(left_at > 0 && right_at > 0);
    assert_eq!(
        left.team
            .space
            .nest
            .physical_at(KVirt::wrap(left_at as usize)),
        right
            .team
            .space
            .nest
            .physical_at(KVirt::wrap(right_at as usize))
    );

    let source = left_hold.source();
    let pending = left
        .team
        .space
        .nest
        .reserve(Placement::lowest(RegionId::Normal, PAGE_SIZE, PAGE_SIZE).unwrap())
        .unwrap();
    let id = pending.id();
    left.team
        .space
        .views
        .claim(
            id,
            &left.team.space,
            left.task.ident.id,
            ViewControl::Caller,
            source.clone(),
            Some(left_token),
        )
        .unwrap();
    source
        .enter_publish(id, left_hold.grant().map(|grant| &**grant), Access::READ)
        .unwrap();
    assert_eq!(
        pend::release(&left.task, left_token),
        Err(env::PieFail::Busy)
    );
    assert!(pend::locate(&left.task, left_token).unwrap().alive());
    assert!(
        left.team
            .space
            .nest
            .translate(KVirt::wrap(left_at as usize))
            .is_some()
    );
    source.leave_publish(id);
    left.team.space.views.cancel(id);
    left.team.space.nest.release_reservation(pending).unwrap();
    pend::release(&left.task, left_token).unwrap();
    assert!(
        left.team
            .space
            .nest
            .translate(KVirt::wrap(left_at as usize))
            .is_none()
    );
    assert!(
        right
            .team
            .space
            .nest
            .translate(KVirt::wrap(right_at as usize))
            .is_some()
    );
    assert!(left_hold.resolve().is_none());
    assert!(right_hold.resolve().is_some());
    assert_eq!(
        left.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(left_ref as usize),
        })),
        0
    );

    let cap = pend::Pie::try_new(
        Mail::Nole(crate::core::work::mail::nole::NoleMeta::mint_retention(
            right.task.ident.id,
        )),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .unwrap();
    let cap_token = cap.token();
    pend::insert(&right.task, cap).unwrap();
    let retained = acquire(&right, right_token, cap_token);
    assert!(retained > 0);
    pend::release(&owner.task, root).unwrap();
    assert!(
        owner
            .team
            .space
            .nest
            .translate(KVirt::wrap(source_at))
            .is_none()
    );
    assert!(
        right
            .team
            .space
            .nest
            .translate(KVirt::wrap(right_at as usize))
            .is_none()
    );
    assert!(right_hold.resolve().is_none());
    assert!(
        source_frame.upgrade().is_some(),
        "independent retention keeps the frame"
    );
    assert!(
        right
            .task
            .pend
            .references
            .get(retained as usize)
            .unwrap()
            .resolve()
            .is_some()
    );
    let scratch = right.scratch_page();
    right.write_words(scratch, &[retained as usize, 0, 1]);
    right.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
    assert_eq!(
        right
            .pair(EnvCall::Memory(MemoryCall::Attach {
                range: VirtAddr::new(scratch),
                team: TeamId::new(0),
                placement: VirtAddr::new(scratch + 32),
                access: 2,
                control: ViewControl::Caller,
            }))
            .0,
        MemoryFail::Denied.code(),
        "retention cannot launder CPU use"
    );
    assert_eq!(
        right.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(retained as usize),
        })),
        0
    );
    assert!(
        source_frame.upgrade().is_none(),
        "ordinary revoked references do not pin frames"
    );
    assert!(right.task.pend.references.get(right_ref as usize).is_some());
    assert_eq!(
        right.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(right_ref as usize),
        })),
        0
    );
    pend::release(&right.task, cap_token).unwrap();
}

pub fn control_and_death() {
    use env::{
        AcquireSource, Allocation, MemoryFail, PieToken, TeamId, ViewControl, ViewSelection,
    };
    crate::core::work::room::scheduler::init().unwrap();
    let owner = Caller::new(false);
    let peer = Caller {
        team: owner.team.clone(),
        task: owner.team.task().hold().unwrap(),
        scratch: core::cell::RefCell::new(None),
        mapped: core::cell::RefCell::new(alloc::vec::Vec::new()),
    };
    let foreign = Caller::new(false);
    let (private, private_at) = owner.attach_alloc(PAGE_SIZE, 6);
    let detach = |id| MemoryCall::Detach {
        team: TeamId::new(0),
        view: view_handle(id),
        selection: ViewSelection::Whole,
    };
    let protect = |id| MemoryCall::SetAccess {
        team: TeamId::new(0),
        view: view_handle(id),
        selection: ViewSelection::Whole,
        access: 2,
    };
    assert_eq!(
        peer.call(EnvCall::Memory(detach(private))),
        MemoryFail::Denied.code()
    );
    assert_eq!(
        peer.call(EnvCall::Memory(protect(private))),
        MemoryFail::Denied.code()
    );
    assert_eq!(
        foreign.call(EnvCall::Memory(detach(private))),
        MemoryFail::Denied.code()
    );
    let frame = |id: usize, at| {
        let entry = owner.team.space.views.locate(id).unwrap();
        let frames = entry
            .view()
            .unwrap()
            .frames_in(KVirt::wrap(at), PAGE_SIZE)
            .unwrap();
        Arc::downgrade(&frames[0].1)
    };
    let private_frame = frame(private, private_at);
    let source = owner.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
        retention: PieToken::NONE,
    }));
    assert!(source > 0);
    owner
        .task
        .pend
        .references
        .get(source as usize)
        .unwrap()
        .resolve()
        .unwrap()
        .materialize()
        .unwrap();
    let scratch = owner.scratch_page();
    owner.write_words(scratch, &[source as usize, 0, 1]);
    owner.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
    let (shared, shared_at) = owner.pair(EnvCall::Memory(MemoryCall::Attach {
        range: VirtAddr::new(scratch),
        team: TeamId::new(0),
        placement: VirtAddr::new(scratch + 32),
        access: 6,
        control: ViewControl::SpaceMembers,
    }));
    assert!(shared > 0);
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(source as usize),
        })),
        0
    );
    let shared_frame = frame(shared as usize, shared_at);
    assert_eq!(peer.call(EnvCall::Memory(protect(shared as usize))), 0);
    assert_eq!(
        foreign.call(EnvCall::Memory(detach(shared as usize))),
        MemoryFail::Denied.code()
    );
    assert_eq!(
        peer.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::View {
                team: TeamId::new(0),
                view: view_handle(shared as usize),
                offset: 0,
                count: 1,
            },
            retention: PieToken::NONE,
        })),
        MemoryFail::Denied.code(),
        "shared control gives no source ownership"
    );
    {
        let _commit = crate::core::work::commit();
        owner.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    }
    pend::doom(&owner.task);
    assert!(owner.team.space.views.locate(private).is_none());
    assert!(
        owner
            .team
            .space
            .nest
            .translate(KVirt::wrap(private_at))
            .is_none()
    );
    assert!(
        private_frame.upgrade().is_none(),
        "Caller data retires with its controller"
    );
    assert!(
        shared_frame.upgrade().is_some(),
        "shared data survives its issuer"
    );
    assert!(
        owner
            .team
            .space
            .validate_access(KVirt::wrap(shared_at), PAGE_SIZE, Access::READ)
    );
    assert_eq!(peer.call(EnvCall::Memory(detach(shared as usize))), 0);
    assert!(shared_frame.upgrade().is_none());
}

/// 独立保留授权只认内核铸的真对象：普通自造 Nole、`Mark`、权限位都不算。
pub fn retention() {
    use crate::core::work::mail::nole::NoleMeta;
    use env::{AcquireSource, Allocation, Mark, PieToken, ReferenceRange};
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    let acquire =
        |source, retention| user.call(EnvCall::Memory(MemoryCall::Acquire { source, retention }));
    let source = acquire(
        AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
        PieToken::NONE,
    );
    assert!(source > 0, "acquire allocation");
    let window = ReferenceRange::new(reference(source as usize), 0, 1).unwrap();

    // 普通自造 Nole：即使拿到同一种权限位也不授权。
    let ordinary = task.pend.pies.lock().len();
    let token = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::try_new(task.ident.id).expect("ordinary nole")),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        task.pend.pies.lock().push(pie);
        token
    };
    assert_eq!(task.pend.pies.lock().len(), ordinary + 1);
    assert_eq!(
        acquire(AcquireSource::Reference(window), token),
        env::MemoryFail::Denied.code(),
        "a user-made Nole is not a retention authority"
    );

    // 内核铸的那一枚：对象本身是凭证，于是同一份来源可以拿到独立保留。
    let trusted = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::mint_retention(task.ident.id)),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        task.pend.pies.lock().push(pie);
        token
    };
    let detached = acquire(AcquireSource::Reference(window), trusted);
    assert!(
        detached > 0,
        "kernel-minted retention authorizes: {detached}"
    );
    // 保留那一份自己强持这段范围（不靠来源授权的生死）。
    let retained = task
        .pend
        .references
        .get(detached as usize)
        .expect("retained hold");
    assert!(
        retained.resolve().is_some(),
        "retention holds its own range"
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(detached as usize),
        })),
        0
    );
}

/// 从线上那一格取一枚关系句柄（外部测试只有解码这一扇门）。
fn view_handle(raw: usize) -> env::ViewId {
    match MemoryCall::from_wire((2usize << 32) | 8, &[0, raw, 0, 0, 0, 0]).unwrap() {
        MemoryCall::Detach { view, .. } => view,
        other => panic!("unexpected detach: {other:?}"),
    }
}

/// 把定长描述写进调用者自己的地址空间。
fn write_words(caller: &Caller, at: usize, words: &[usize]) {
    let mut bytes = [0u8; 32];
    for (index, word) in words.iter().enumerate() {
        bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_ne_bytes());
    }
    assert!(
        caller
            .team
            .space
            .copy_out(&bytes[..words.len() * 8], KVirt::wrap(at))
    );
}

/// `Attach` / `Detach` / `SetAccess`：真实入口、双值返回与控制范围。
pub fn attach_detach_access() {
    use env::{
        AcquireSource, Allocation, MemoryFail, PieToken, TeamId, ViewControl, ViewSelection,
    };

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    // 先要一页可写的用户内存放描述（旧 ABI 仍在，用作引导）。
    let scratch = user.allocate(PAGE_SIZE);
    let hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
        retention: PieToken::NONE,
    }));
    assert!(hold > 0, "acquire: {hold}");
    // 物化是 FlatRange 自己的原语（不进 ABI）：先物化再挂，页才在。
    task.pend
        .references
        .get(hold as usize)
        .expect("hold")
        .source()
        .own_range()
        .expect("private range")
        .materialize()
        .expect("materialize");
    let (range_at, placement_at) = (scratch, scratch + 32);
    write_words(&user, range_at, &[hold as usize, 0, 1]);
    write_words(&user, placement_at, &[0, 0, PAGE_SIZE]);

    let (id, at) = user.pair(EnvCall::Memory(MemoryCall::Attach {
        range: VirtAddr::new(range_at),
        team: TeamId::new(0),
        placement: VirtAddr::new(placement_at),
        access: 6,
        control: ViewControl::Caller,
    }));
    assert!(id > 0, "attach: {id}");
    assert!(user.team.space.copy_out(b"newabi", KVirt::wrap(at)));
    let entry = user
        .team
        .space
        .views
        .locate(id as usize)
        .expect("registered view");
    assert_eq!(entry.controller, user.task.ident.id);

    // 收窄到只读：写不再放行，读仍在。
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::SetAccess {
            team: TeamId::new(0),
            view: view_handle(id as usize),
            selection: ViewSelection::Whole,
            access: 2,
        })),
        0
    );
    assert!(!user.team.space.copy_out(b"x", KVirt::wrap(at)));
    let mut bytes = [0u8; 6];
    assert!(user.team.space.copy_in(&mut bytes, KVirt::wrap(at)));
    assert_eq!(&bytes, b"newabi");

    // 另一段号猜不动：同号不存在就是 Denied。
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::SetAccess {
            team: TeamId::new(0),
            view: view_handle(isize::MAX as usize),
            selection: ViewSelection::Whole,
            access: 2,
        })),
        MemoryFail::Denied.code()
    );

    // 撤销选定的映射区间，最后一段清完才回收号。
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: TeamId::new(0),
            view: view_handle(id as usize),
            selection: ViewSelection::Whole,
        })),
        0
    );
    assert!(user.team.space.views.locate(id as usize).is_none());
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: TeamId::new(0),
            view: view_handle(id as usize),
            selection: ViewSelection::Whole,
        })),
        MemoryFail::Denied.code()
    );
}

/// gate 的交错回归：在途发布未结清时撤销必须 Busy；结清后撤销成功，
/// 撤销后的新发布被拒，已发布的关系仍由来源收得回（不漏发布）。
pub fn gate_interleaving() {
    use env::{
        AcquireSource, Allocation, MemoryFail, PieFail, PieToken, TeamId, ViewControl,
        ViewSelection,
    };

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    let scratch = user.allocate(PAGE_SIZE);
    let hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Allocation(Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap()),
        retention: PieToken::NONE,
    }));
    assert!(hold > 0);
    let authorization = task
        .pend
        .references
        .get(hold as usize)
        .expect("hold")
        .source()
        .clone();
    authorization
        .own_range()
        .expect("private range")
        .materialize()
        .expect("materialize");
    let (range_at, placement_at) = (scratch, scratch + 32);

    let attach = |at: usize| {
        user.pair(EnvCall::Memory(MemoryCall::Attach {
            range: VirtAddr::new(at),
            team: TeamId::new(0),
            placement: VirtAddr::new(placement_at),
            access: 6,
            control: ViewControl::Caller,
        }))
    };
    write_words(&user, range_at, &[hold as usize, 0, 1]);
    write_words(&user, placement_at, &[0, 0, PAGE_SIZE]);
    let (id, _) = attach(range_at);
    assert!(id > 0, "first attach: {id}");

    // 在途发布：撤销不得先成功再漏发布。
    assert!(
        authorization
            .enter_publish(usize::MAX, None, Access::READ)
            .is_ok()
    );
    assert_eq!(authorization.revoke().err(), Some(PieFail::Busy));
    // 已发布的那一段仍由这份来源收得回，撤销扫得到它。
    let owned = user.team.space.views.owned_by(&authorization).unwrap();
    assert_eq!(owned.len(), 1, "published view is visible to the revoker");
    // 结清在途后撤销成功。
    authorization.leave_publish(usize::MAX);
    assert!(authorization.revoke().is_ok());

    // 撤销后的新发布：闸门前段就拒，不留半句柄。
    let (again, _) = attach(range_at);
    assert_eq!(again, MemoryFail::Denied.code());

    // 收尾：已发布的关系由撤销方锁外关，号一并收回。
    for (held, entry) in owned {
        user.team
            .space
            .nest
            .dispose(entry.view().expect("published").clone())
            .expect("dispose");
        user.team.space.views.retire(held);
    }
    assert!(
        user.team
            .space
            .views
            .owned_by(&authorization)
            .unwrap()
            .is_empty()
    );
    let _ = ViewSelection::Whole;
}

pub fn capability() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let foreign = Caller::new(false);
    let meta = hole::HoleMeta::new(user.task.ident.id);
    let pie = pend::Pie::try_new(
        Mail::Hole(meta.clone()),
        env::Mark::NONE,
        Permission::FETCH,
        None,
    )
    .expect("pie allocation");
    let token = pie.token();
    user.task.pend.pies.lock().push(pie);
    assert!(pend::accede(&user.task, token, Need::Fetch).is_ok());
    assert!(matches!(
        pend::accede(&user.task, token, Need::Store),
        Err(env::PieFail::Denied)
    ));
    assert!(matches!(
        pend::accede(&foreign.task, token, Need::Fetch),
        Err(env::PieFail::Denied)
    ));
    assert!(matches!(
        pend::accede(&user.task, env::PieToken::mint(usize::MAX), Need::Fetch),
        Err(env::PieFail::Denied)
    ));
    hole::seal(&meta);
    assert!(matches!(
        pend::accede(&user.task, token, Need::Fetch),
        Err(env::PieFail::Dead)
    ));
    let (_, _, reference) = user.attach_alloc_reference(PAGE_SIZE, 6);
    let token = crate::entry::call::pie::create_pole(&user.task, reference, true).unwrap();
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release { reference })),
        0
    );
    pend::reduce(&user.task, token, Permission::FETCH).unwrap();
    let pole = pend::locate(&user.task, token)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    let (at, size) = pole::open(&pole, token, &user.team.space, Access::READ).unwrap();
    for (slot, regs) in [
        (1, [at, size, 0, 0, 0, 0]),
        (3, [0, at, size, 0, 0, 0]),
        (4, [0, at, size, 6, 0, 0]),
    ] {
        assert_eq!(
            user.raw((2usize << 32) | slot, regs),
            env::DispatchFail::Unknown.code()
        );
    }
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(at), size, Access::READ)
    );
    let operation = pole::PoleOperation::acquire(&pole).expect("pole operation lease");
    assert_eq!(pole::shut(&pole, token), Err(env::PieFail::Busy));
    drop(operation);
    pole::shut(&pole, token).unwrap();
    assert!(
        !user
            .team
            .space
            .validate_access(KVirt::wrap(at), size, Access::READ)
    );
}

/// 线上 Mmap 的来源路径：Pole 授权、来源窗口、关闭后重挂与借用来源的拒绝。
///
/// 装载器的真实次序是「先 shut 掉 token 的全部 View，再从 Pole 来源 Mmap」，
/// 所以这里按同一次序验证，不靠任何 keeper / canonical 关系。
pub fn memory_pole() {
    use crate::core::memory::flat::Frame;
    use crate::entry::call::pie::create_pole;
    use env::{MemoryFail, TeamId};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let size = PAGE_SIZE;
    let (_, source, reference) = user.attach_alloc_reference(size, 6);
    let token = create_pole(&user.task, reference, true).unwrap();
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release { reference })),
        0
    );
    let snapshot = pend::locate(&user.task, token).unwrap();
    let meta = snapshot.mail().pole().unwrap();
    let physical = user
        .team
        .space
        .nest
        .physical_at(KVirt::wrap(source))
        .unwrap();
    let map = |pole: env::PieToken, offset: usize, flags: u64| {
        user.map(env::TeamId::new(0), 0, size, pole, offset, flags)
    };

    // 关闭全部 View 之后来源仍在：装载器就是从这一步开始 Mmap 的。
    pole::shut(&meta, token).unwrap();
    assert!(
        !user
            .team
            .space
            .validate_access(KVirt::wrap(source), size, Access::READ)
    );
    let at = map(token, 0, 6);
    assert!(at > 0, "pole mmap after shut was rejected: {at}");
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(at as usize), size, Access::READ)
    );
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(at as usize), size, Access::WRITE)
    );
    assert_eq!(
        user.team.space.nest.physical_at(KVirt::wrap(at as usize)),
        Some(physical)
    );

    // 字节包装拒不对齐，来源窗口拒越界。
    assert_eq!(map(token, 1, 6), MemoryFail::NotAligned.code() as isize);
    assert_eq!(map(token, size, 6), MemoryFail::NoRegion.code() as isize);

    // 收窄授权上限：写请求拒，只读请求仍成立且交付只读关系。
    pend::reduce(&user.task, token, Permission::FETCH | Permission::VEST).unwrap();
    assert_eq!(map(token, 0, 6), MemoryFail::Denied.code() as isize);
    let readonly = map(token, 0, 2);
    assert!(readonly > 0, "readonly pole mmap was rejected: {readonly}");
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(readonly as usize), size, Access::READ)
    );
    assert!(
        !user
            .team
            .space
            .validate_access(KVirt::wrap(readonly as usize), size, Access::WRITE)
    );

    // 借用（设备）来源不算自有 RAM：旧共享授权拒绝，设备访问走 Pie Open。
    let frame = Frame::allocate().unwrap();
    let borrowed = pole::region(frame.physical_address(), size, user.task.ident.id).unwrap();
    let pie = pend::Pie::try_new(
        Mail::Pole(borrowed.clone()),
        env::Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .unwrap();
    let device = pie.token();
    user.task.pend.pies.lock().push(pie);
    assert_eq!(map(device, 0, 2), MemoryFail::Denied.code() as isize);
    let (va, _) = pole::open(&borrowed, device, &user.team.space, Access::READ).unwrap();
    assert!(
        user.team
            .space
            .validate_access(KVirt::wrap(va), size, Access::READ)
    );

    // 构造中的队伍：没有 VEST 不接，有 VEST 才按定点地址交付来源页。
    let build = pend::Pie::try_new(
        Mail::Nole(crate::entry::call::resources::get().build.clone()),
        env::Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .unwrap();
    user.task.pend.pies.lock().push(build);
    let child = TeamId::new(user.call(EnvCall::Unit(UnitCall::Build {
        kind: ProgramKind::User,
        sire: user.task.ident.id,
    })) as usize);
    assert!(
        user.task.clan(child).is_some(),
        "built team is not a clan of the caller"
    );
    let mint =
        |pole: env::PieToken, at: usize, flags: u64| user.map(child, at, size, pole, 0, flags);
    let (_, fresh, fresh_reference) = user.attach_alloc_reference(size, 6);
    assert!(user.team.space.nest.translate(KVirt::wrap(fresh)).is_some());
    let owned = create_pole(&user.task, fresh_reference, true).unwrap();
    pend::reduce(&user.task, owned, Permission::FETCH | Permission::STORE).unwrap();
    assert_eq!(
        mint(owned, 0x4000_0000, 2),
        MemoryFail::Denied.code() as isize
    );
    let staged = mint(token, 0x4000_0000, 2);
    assert!(
        staged > 0,
        "vest pole mmap into a constructing team: {staged}"
    );
    assert_eq!(user.call(EnvCall::Unit(UnitCall::Oust { team: child })), 0);

    drop(snapshot);
    drop(meta);
}

pub fn resource_registration() {
    use crate::entry::boot::resources::Registry;
    use crate::entry::{call::resources as calls, trap::resources as traps};
    use env::{Call, Mark, Name, PieFail, PieToken, Trap};
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let system = Caller::new(true);
    let service = Caller::new(false);
    let other = Caller::new(false);
    let mut registry = Registry::default();
    calls::register(&mut registry).unwrap();
    traps::register(&mut registry).unwrap();
    let root = || {
        pend::Pie::try_new(
            Mail::Nole(calls::get().build.clone()),
            Mark::NONE,
            Permission::FETCH | Permission::VEST,
            None,
        )
        .expect("pie allocation")
    };
    let duplicate = root();
    let token = duplicate.token();
    let error = registry
        .register(Name::Call(Call::Build), duplicate)
        .unwrap_err();
    assert_eq!(error.reason, PieFail::Denied);
    assert_eq!(error.value.1.token(), token);
    assert!(
        registry
            .register(Name::Trap(Trap::PageFault), root())
            .is_err()
    );
    let entries = registry.freeze().unwrap().grant(&system.task).unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(env::Entry::valid));
    let token = entries
        .iter()
        .find(|e| e.name() == Name::Call(Call::Build))
        .unwrap()
        .token();
    assert!(pend::allows(
        &system.task,
        &Mail::Nole(calls::get().build.clone()),
        Need::Fetch,
    ));
    let token = PieToken::mint(
        pend::accord(
            &system.task,
            token,
            &Arc::downgrade(&service.task),
            Permission::FETCH,
            Mark::NONE,
        )
        .unwrap(),
    );
    assert!(pend::allows(
        &service.task,
        &Mail::Nole(calls::get().build.clone()),
        Need::Fetch
    ));
    assert!(
        service.call(EnvCall::Unit(UnitCall::Build {
            kind: ProgramKind::User,
            sire: service.task.ident.id
        })) > 0
    );
    assert!(matches!(
        pend::accord(
            &service.task,
            token,
            &Arc::downgrade(&other.task),
            Permission::FETCH,
            Mark::NONE,
        ),
        Err(PieFail::Denied)
    ));
    pend::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    assert!(!pend::allows(
        &service.task,
        &Mail::Nole(calls::get().build.clone()),
        Need::Fetch
    ));
    assert_eq!(
        service.call(EnvCall::Unit(UnitCall::Build {
            kind: ProgramKind::User,
            sire: service.task.ident.id
        })),
        env::UnitFail::Denied.code()
    );
    let doom = entries
        .iter()
        .find(|e| e.name() == Name::Call(Call::Doom))
        .unwrap()
        .token();
    let call = EnvCall::Room(RoomCall::Doom {
        task: env::TaskId::new(usize::MAX),
    });
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    let token = PieToken::mint(
        pend::accord(
            &system.task,
            doom,
            &Arc::downgrade(&service.task),
            Permission::VEST,
            Mark::NONE,
        )
        .unwrap(),
    );
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    pend::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    let token = PieToken::mint(
        pend::accord(
            &system.task,
            doom,
            &Arc::downgrade(&service.task),
            Permission::FETCH,
            Mark::NONE,
        )
        .unwrap(),
    );
    assert!(pend::allows(
        &service.task,
        &Mail::Nole(calls::get().doom.clone()),
        Need::Fetch,
    ));
    let victim = Caller::new(false);
    assert_eq!(
        service.call(EnvCall::Room(RoomCall::Doom {
            task: victim.task.ident.id,
        })),
        0
    );
    assert!(victim.task.dead());
    assert_eq!(service.call(call), env::RoomFail::Dead.code());
    assert!(matches!(
        pend::accord(
            &service.task,
            token,
            &Arc::downgrade(&other.task),
            Permission::FETCH,
            Mark::NONE,
        ),
        Err(PieFail::Denied)
    ));
    pend::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    let mut registry = Registry::default();
    let dead = crate::core::work::mail::nole::NoleMeta::new(env::TaskId::new(0));
    let invalid = pend::Pie::try_new(
        Mail::Nole(dead.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .expect("pie allocation");
    registry.register(Name::Call(Call::Build), invalid).unwrap();
    crate::core::work::mail::nole::seal(&dead);
    let frozen = match registry.freeze() {
        Err(e) => e,
        Ok(_) => panic!("accepted dead root"),
    };
    assert_eq!(frozen.reason, PieFail::Denied);

    let dead = crate::core::work::mail::nole::NoleMeta::new(env::TaskId::new(0));
    let mut registry = Registry::default();
    let invalid = pend::Pie::try_new(
        Mail::Nole(dead.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )
    .expect("pie allocation");
    registry.register(Name::Call(Call::Build), invalid).unwrap();
    let frozen = registry.freeze().unwrap();
    crate::core::work::mail::nole::seal(&dead);
    let before = other.task.pend.pies.lock().len();
    let error = frozen.grant(&other.task).unwrap_err();
    assert_eq!(error.reason, PieFail::Denied);
    assert_eq!(error.value.len(), 1);
    assert_eq!(other.task.pend.pies.lock().len(), before);
}

pub fn transfer_relations() {
    use alloc::vec::Vec;
    use env::{Mark, PieFail, PieToken};
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let a = Caller::new(false);
    let b = Caller::new(false);
    let c = Caller::new(false);
    let permission = Permission::FETCH | Permission::STORE | Permission::VEST;
    let root = |task: &Arc<Task>, permission| {
        let pie = pend::Pie::try_new(
            Mail::Nole(crate::core::work::mail::nole::NoleMeta::new(task.ident.id)),
            Mark::NONE,
            permission,
            None,
        )
        .expect("pie allocation");
        let token = pie.token();
        pend::insert(task, pie).unwrap();
        token
    };
    let grant = |task: &Arc<Task>, token, target: &Arc<Task>, permission| {
        PieToken::mint(
            pend::accord(task, token, &Arc::downgrade(target), permission, Mark::NONE).unwrap(),
        )
    };
    let token = root(&a.task, permission);
    let ab = grant(&a.task, token, &b.task, permission);
    let bc = grant(&b.task, ab, &c.task, permission);
    assert_eq!(pend::vestor(&c.task, bc), Some(b.task.ident.id));
    pend::forget(&b.task, ab).unwrap();
    assert_eq!(pend::vestor(&c.task, bc), Some(a.task.ident.id));
    assert!(b.task.pend.pies.lock().is_empty());
    assert!(b.task.pend.heirs.lock().is_empty());
    assert_eq!(pend::release(&a.task, token), Ok(2));
    assert!(c.task.pend.pies.lock().is_empty());

    let token = root(&a.task, permission);
    let child = grant(&a.task, token, &a.task, permission);
    assert_eq!(pend::vestor(&a.task, child), Some(a.task.ident.id));
    assert_eq!(pend::release(&a.task, token), Ok(2));

    let sole = permission | Permission::ONLY;
    let token = root(&a.task, sole);
    let ab = grant(&a.task, token, &b.task, sole);
    let old = pend::locate(&a.task, token)
        .unwrap()
        .heir()
        .copied()
        .unwrap();
    pend::revoke(&a.task, &Arc::downgrade(&b.task), ab).unwrap();
    let next = grant(&a.task, token, &b.task, sole);
    assert!(!pend::clear_heir(&a.task, token, old));
    assert_eq!(
        pend::locate(&a.task, token).unwrap().heir().unwrap().token,
        next
    );
    pend::release(&a.task, token).unwrap();

    let children: Vec<_> = (0..16).map(|_| Caller::new(false)).collect();
    let token = root(&a.task, permission);
    for child in &children {
        grant(&a.task, token, &child.task, permission);
    }
    assert_eq!(pend::release(&a.task, token), Ok(17));
    assert!(
        children
            .iter()
            .all(|child| child.task.pend.pies.lock().is_empty())
    );

    let d = Caller::new(false);
    let (_, _, reference) = a.attach_alloc_reference(PAGE_SIZE, 6);
    let memory = crate::entry::call::pie::create_pole(&a.task, reference, true).unwrap();
    assert_eq!(
        a.call(EnvCall::Memory(MemoryCall::Release { reference })),
        0
    );
    let meta = pend::locate(&a.task, memory)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    let ad = grant(&a.task, memory, &d.task, permission);
    grant(&d.task, ad, &c.task, permission);
    {
        let _operation = pole::PoleOperation::acquire(&meta).unwrap();
        {
            let _commit = crate::core::work::commit();
            d.task.state.lock().doom(None, env::ExitCause::Slay, 0);
        }
        pend::doom(&d.task);
    }
    assert!(d.task.pend.pies.lock().is_empty());
    assert!(c.task.pend.pies.lock().is_empty());

    let token = root(&a.task, permission);
    let ab = grant(&a.task, token, &b.task, permission);
    grant(&b.task, ab, &c.task, permission);
    {
        let _commit = crate::core::work::commit();
        b.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    }
    pend::doom(&b.task);
    assert!(b.task.pend.pies.lock().is_empty());
    assert!(c.task.pend.pies.lock().is_empty());
    assert!(a.task.pend.heirs.lock().is_empty());
    assert_eq!(
        pend::accord(
            &a.task,
            token,
            &Arc::downgrade(&b.task),
            permission,
            Mark::NONE
        ),
        Err(PieFail::Dead)
    );
}

pub fn unit_scan_and_receipts() {
    use crate::core::work::unit::copies::Origin;
    use crate::core::work::unit::task::{TaskState, TaskWeak};
    use alloc::vec::Vec;
    use env::{UnitFail, UnitTarget, Wait};
    crate::core::work::room::scheduler::init().unwrap();
    let caller = Caller::new(false);
    let mut children = Vec::new();
    for _ in 0..73 {
        let space = SpaceBuilder::user().build().unwrap();
        space
            .nest
            .attach_region(RegionId::Normal, 0x4000_0000, mode::upper().as_usize())
            .unwrap();
        children.push(
            TeamBuilder::new(space)
                .sire(TaskWeak::stored(Arc::downgrade(&caller.task), Origin::Sire))
                .spawn()
                .unwrap(),
        );
    }
    let buffer = caller.allocate(PAGE_SIZE);
    let scan = |after, capacity| {
        EnvCall::Unit(UnitCall::Scan {
            after,
            buf: VirtAddr::new(buffer),
            capacity,
        })
    };
    assert_eq!(
        caller.call(scan(env::TeamId::new(0), 0)),
        UnitFail::Denied.code()
    );
    assert_eq!(
        caller.call(scan(env::TeamId::new(0), 65)),
        UnitFail::Denied.code()
    );
    assert_eq!(caller.call(scan(env::TeamId::new(0), 64)), 64);
    let mut page = [0u8; 512];
    assert!(caller.team.space.copy_in(&mut page, KVirt::wrap(buffer)));
    let ids: Vec<_> = page
        .chunks_exact(8)
        .map(|w| u64::from_le_bytes(w.try_into().unwrap()) as usize)
        .collect();
    assert!(ids.windows(2).all(|p| p[0] < p[1]));
    assert_eq!(ids[0], children[0].id.get());
    assert_eq!(caller.call(scan(env::TeamId::new(ids[63]), 64)), 9);
    assert_eq!(caller.call(scan(children[72].id, 64)), 0);
    let target = &children[0];
    let task = target.task().hold().unwrap();
    let observe = EnvCall::Unit(UnitCall::Join {
        target: UnitTarget::Task(task.ident.id),
        millis: Wait::POLL,
        receive: false,
    });
    assert_eq!(caller.call(observe), 0);
    assert_eq!(
        caller.call(EnvCall::Unit(UnitCall::Oust { team: target.id })),
        UnitFail::Busy.code()
    );
    target.release_held(&task);
    *task.state.lock() = TaskState::reaped(env::ExitCause::Reap, 17);
    crate::core::work::unit::tally::exit();
    assert_eq!(caller.call(observe), env::ExitCause::Reap as isize);
    assert_eq!(caller.call(observe), env::ExitCause::Reap as isize);
    assert_eq!(
        caller.call(EnvCall::Unit(UnitCall::Join {
            target: UnitTarget::Task(task.ident.id),
            millis: Wait::POLL,
            receive: true,
        })),
        env::ExitCause::Reap as isize
    );
    let frame = unsafe { &*(caller.task.ident.frame_pa.as_usize() as *const TrapContext) };
    assert_eq!(frame.gpr.x(Gprs::A1), task.ident.id.get());
    assert_eq!(frame.gpr.x(Gprs::A2), 17);
    assert_eq!(
        caller.call(EnvCall::Unit(UnitCall::Join {
            target: UnitTarget::Task(task.ident.id),
            millis: Wait::POLL,
            receive: true,
        })),
        UnitFail::Denied.code()
    );
    for child in &children {
        assert_eq!(
            caller.call(EnvCall::Unit(UnitCall::Oust { team: child.id })),
            0
        );
    }
    assert_eq!(caller.call(scan(env::TeamId::new(0), 64)), 0);
}

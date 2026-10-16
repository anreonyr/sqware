#![cfg(debug_assertions)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use env::{ControlCall, DebugCall, EnvCall, MemoryCall, ProgramKind, RoomCall, UnitCall, VirtAddr};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr as KVirt};
use crate::core::memory::flat::{Flat, FlatRange};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, Asid, NestError, Placement, RegionId, View};
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
use crate::machine::sync::SpinLock;

#[path = "retention/mod.rs"]
mod external_retention;
pub use external_retention::accept as external_retention;

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

/// 借来的设备区也能**按子窗口**取：引导期给的授权记录让持有者只要自己那一小段。
///
/// 这就是 BAR 那一格的真实通路：不需要把整段 aperture（或整段 ECAM）攞进本域。
pub fn device_window() {
    use crate::core::memory::flat::Frame;
    use crate::core::work::mail::pole;
    use crate::core::work::pend::SourceAuthorization;
    use env::{AcquireSource, Mark, MemoryFail, PieToken, ReferenceRange};
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    let frame = Frame::allocate().expect("frame");
    let physical = frame.physical_address();
    let meta = pole::region(physical, 2 * PAGE_SIZE, task.ident.id).expect("region");
    let pie = pend::Pie::try_new(
        Mail::Pole(meta.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .expect("pie");
    let token = pie.token();
    task.pend.pies.lock().push(pie);
    let authorization =
        SourceAuthorization::new(meta.source(), Access::all(), task.ident.id).expect("authority");
    meta.attach_source(&authorization);
    assert!(
        authorization
            .adopt(&meta, token, meta.grant(token).expect("grant"))
            .is_ok()
    );

    // 越出这段区的窗口先拒。
    let outside = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token,
            offset: 2,
            count: 1,
        },
        retention: PieToken::NONE,
    }));
    assert_eq!(outside, MemoryFail::NoRegion.code());

    let hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token,
            offset: 1,
            count: 1,
        },
        retention: PieToken::NONE,
    }));
    assert!(hold > 0, "window acquire: {hold}");
    let scratch = user.allocate(PAGE_SIZE);
    let range = ReferenceRange::new(reference(hold as usize), 0, 1).expect("range");
    write_words(&user, scratch, &[range.reference.get(), 0, 1]);
    write_words(&user, scratch + 32, &[0, 0, PAGE_SIZE]);
    let (id, at) = user.pair(EnvCall::Memory(MemoryCall::Attach {
        range: VirtAddr::new(scratch),
        team: env::TeamId::new(0),
        placement: VirtAddr::new(scratch + 32),
        access: 6,
        control: env::ViewControl::Caller,
    }));
    assert!(id > 0, "window attach: {id}");
    assert_eq!(at % PAGE_SIZE, 0);
    assert_eq!(
        user.team
            .space
            .nest
            .physical_at(KVirt::wrap(at))
            .expect("mapped page")
            .as_usize(),
        physical + PAGE_SIZE,
        "the window keeps its own coordinates inside the borrowed region"
    );
    // 借用区只开 MMIO 读写：执行映射要拒。
    assert_eq!(
        user.pair(EnvCall::Memory(MemoryCall::Attach {
            range: VirtAddr::new(scratch),
            team: env::TeamId::new(0),
            placement: VirtAddr::new(scratch + 32),
            access: 6 | 8,
            control: env::ViewControl::Caller,
        }))
        .0,
        MemoryFail::Denied.code(),
        "a device window does not hand out execute mappings"
    );
    // 清除这一格：自己建的那张映射由自己拆，不留给任务尽头。
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Detach {
            team: env::TeamId::new(0),
            view: view_handle(id as usize),
            selection: env::ViewSelection::Whole,
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(hold as usize),
        })),
        0
    );
    assert_eq!(user.team.space.nest.physical_at(KVirt::wrap(at)), None);
}

/// Traverse exports only a retained window under current authority.
///
/// 三条判据：保留那一份给得出对得上真帧的起址；普通引用一律拒；
/// 两次取到的起址是同两页（同一批帧，不是每次另发一份）。
pub fn traverse_pages() {
    use crate::core::work::mail::nole::NoleMeta;
    use env::{AcquireSource, Allocation, Mark, MemoryFail, PieToken, ReferenceRange};
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let task = &user.task;
    let acquire =
        |source, retention| user.call(EnvCall::Memory(MemoryCall::Acquire { source, retention }));
    let ordinary = acquire(
        AcquireSource::Allocation(Allocation::new(2 * PAGE_SIZE, PAGE_SIZE).unwrap()),
        PieToken::NONE,
    );
    assert!(ordinary > 0, "acquire allocation");
    let window = ReferenceRange::new(reference(ordinary as usize), 0, 2).unwrap();
    let scratch = user.allocate(PAGE_SIZE);

    // 普通引用拒：物理坐标不随不带保留的引用走（连不带 cap 的那一路也拒）。
    write_words(&user, scratch, &[ordinary as usize, 0, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: PieToken::NONE,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::Denied.code(),
        "an ordinary hold has no physical coordinates to give"
    );

    let trusted = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::mint_retention(task.ident.id).expect("retention cap")),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        task.pend.pies.lock().push(pie);
        token
    };
    let retained = acquire(AcquireSource::Reference(window), trusted);
    assert!(retained > 0, "kernel-minted retention authorizes");

    // 有活 cap，但引用是**普通**那一枚：仍然拒。
    // （上面两条走的是 cap 那一关，“这条”走的是 retained 那一关。）
    write_words(&user, scratch, &[ordinary as usize, 0, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::Denied.code(),
        "a live cap does not lend an ordinary reference physical coordinates"
    );
    let hold = task.pend.references.get(retained as usize).expect("hold");
    let exported = hold.resolve().expect("retained source");
    let pages = hold
        .resolve()
        .expect("retained range")
        .materialize()
        .expect("materialize")
        .page_count()
        .get();
    assert_eq!(pages, 2);
    write_words(&user, scratch, &[retained as usize, 0, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: PieToken::NONE,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::Denied.code(),
        "provenance alone is not a live authority: the call must present the cap"
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        0,
        "a live retention cap plus this task's own retained window answers"
    );
    let mut bytes = [0u8; 16];
    assert!(
        user.team
            .space
            .copy_in(&mut bytes, KVirt::wrap(scratch + 64))
    );
    let list = [
        usize::from_ne_bytes(bytes[..8].try_into().unwrap()),
        usize::from_ne_bytes(bytes[8..].try_into().unwrap()),
    ];
    assert!(
        list.iter().all(|address| address % PAGE_SIZE == 0),
        "physical starts are page aligned: {list:?}"
    );
    let frames: alloc::vec::Vec<_> = hold
        .resolve()
        .unwrap()
        .materialized_frames()
        .unwrap()
        .into_iter()
        .map(|(_, frame)| frame.physical_address())
        .collect();
    assert_eq!(
        list.to_vec(),
        frames,
        "the answers are those very frames, not a fresh set"
    );

    // 别的一枚 cap（属主是别人）不能替它开门。
    let foreign = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::mint_retention(env::TaskId::new(0x1234)).expect("cap")),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        task.pend.pies.lock().push(pie);
        token
    };
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: foreign,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::Denied.code(),
        "the cap must be the calling task's own"
    );

    // 输出跑到末尾未映射的那一页：整段先验后拷，前缀哨兵不动。
    let edge = scratch + PAGE_SIZE - 4;
    assert!(user.team.space.copy_out(b"KEEP", KVirt::wrap(edge)));
    write_words(&user, scratch, &[retained as usize, 0, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(edge),
        })),
        MemoryFail::Denied.code(),
        "an output that runs off the mapped page is refused whole"
    );
    let mut sentinel = [0u8; 4];
    assert!(user.team.space.copy_in(&mut sentinel, KVirt::wrap(edge)));
    assert_eq!(&sentinel, b"KEEP", "no half list before the failure");

    // 出口那一步注入失败：前缀哨兵不动、引用还在。
    assert!(user.team.space.copy_out(b"KEEP", KVirt::wrap(scratch + 64)));
    write_words(&user, scratch, &[retained as usize, 0, 2]);
    crate::entry::call::memory::fail_prepare_at(11);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::OoM.code(),
        "a preparation failure must not write half the table"
    );
    let mut held_sentinel = [0u8; 4];
    assert!(
        user.team
            .space
            .copy_in(&mut held_sentinel, KVirt::wrap(scratch + 64))
    );
    assert_eq!(
        &held_sentinel, b"KEEP",
        "prefix untouched after the injected failure"
    );
    assert!(task.pend.references.get(retained as usize).is_some());
    write_words(&user, scratch, &[retained as usize, 0, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        0,
        "the same call succeeds once the checkpoint is consumed"
    );

    // 描述里的越界窗口：checked 口径先拒（offset + count 溢出）。
    write_words(&user, scratch, &[retained as usize, usize::MAX, 2]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::NotAligned.code(),
        "a descriptor whose window overflows checked coordinates is refused"
    );
    // 格式认得但落在这份持有之外：拒。
    write_words(&user, scratch, &[retained as usize, 3, 1]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::NoRegion.code(),
        "a window past the hold is refused"
    );

    // 保留的是**子范围**时，导出的就是子范围那一页。
    let tail = acquire(
        AcquireSource::Reference(ReferenceRange::new(reference(ordinary as usize), 1, 1).unwrap()),
        trusted,
    );
    assert!(tail > 0, "retained sub-window: {tail}");
    write_words(&user, scratch, &[tail as usize, 0, 1]);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: trusted,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        0
    );
    let mut one = [0u8; 8];
    assert!(user.team.space.copy_in(&mut one, KVirt::wrap(scratch + 64)));
    assert_eq!(
        usize::from_ne_bytes(one),
        frames[1],
        "a retained sub-window answers that page's own frame"
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(tail as usize),
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(retained as usize),
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(ordinary as usize),
        })),
        0
    );
    assert!(
        exported.is_pinned(),
        "the retained snapshot still owns its residency"
    );
    drop(hold);
    assert!(
        !exported.is_pinned(),
        "Traverse success and failure must not leak a hidden residency"
    );
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
        Mail::Nole(
            crate::core::work::mail::nole::NoleMeta::mint_retention(right.task.ident.id)
                .expect("retention cap"),
        ),
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
    // 来源那一枚已经从表里摘掉：同一手上再来一遍（重复/迟到的通知）也不动保留。
    assert_eq!(
        pend::release(&owner.task, root),
        Err(env::PieFail::Denied),
        "a repeated release of the gone source reports, not silent"
    );
    assert!(
        source_frame.upgrade().is_some(),
        "late and repeated notifications never release the retained frame"
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
            Mail::Nole(NoleMeta::mint_retention(task.ident.id).expect("retention cap")),
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
    // 引导那一枚独立保留授权：与上面那批同走一条交付路，只发给这一个任务。
    let cap = crate::entry::boot::resources::grant_retention(&system.task).unwrap();
    assert_eq!(cap.name(), Name::Call(Call::Retain));
    assert_eq!(cap.kind(), env::PieKind::Nole);
    let acquire = |caller: &Caller, source, retention| {
        caller.call(EnvCall::Memory(MemoryCall::Acquire { source, retention }))
    };
    let allocation =
        || env::AcquireSource::Allocation(env::Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap());
    let source = acquire(&system, allocation(), env::PieToken::NONE);
    assert!(source > 0, "ordinary acquire: {source}");
    let window = env::ReferenceRange::new(reference(source as usize), 0, 1).unwrap();
    let held = acquire(&system, env::AcquireSource::Reference(window), cap.token());
    assert!(
        held > 0,
        "the trusted controller's own cap authorizes: {held}"
    );
    // 同一枚 cap 转到别人手里也不授权：保留只认铸给谁的那一个 owner。
    let shared = PieToken::mint(
        pend::accord(
            &system.task,
            cap.token(),
            &Arc::downgrade(&service.task),
            Permission::FETCH | Permission::VEST,
            Mark::NONE,
        )
        .unwrap(),
    );
    assert!(pend::locate(&service.task, shared).is_some());
    let service_source = acquire(&service, allocation(), env::PieToken::NONE);
    assert!(service_source > 0);
    let window = env::ReferenceRange::new(reference(service_source as usize), 0, 1).unwrap();
    assert_eq!(
        acquire(&service, env::AcquireSource::Reference(window), shared),
        env::MemoryFail::Denied.code(),
        "a handed-on retention cap is not the holder's own"
    );
    for (caller, handle) in [
        (&system, source),
        (&system, held),
        (&service, service_source),
    ] {
        assert_eq!(
            caller.call(EnvCall::Memory(MemoryCall::Release {
                reference: reference(handle as usize),
            })),
            0
        );
        assert!(
            caller.task.pend.references.get(handle as usize).is_none(),
            "every handle this case took is settled"
        );
    }
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

// ---- 移交（slot 10）：入口级管理授权、两端窗口与清单的唯一消费者 ----

/// 在途探针的上下文与读数：调用的那几个时刻，调用任务自己的 pend 在不在手上。
static IN_FLIGHT_TASK: SpinLock<Option<Arc<Task>>> = SpinLock::new(None);
static PEND_HELD_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static WAKE_TASK: SpinLock<Option<Arc<Task>>> = SpinLock::new(None);
static PEND_FREE_AT_WAKE: AtomicBool = AtomicBool::new(false);
static WOKEN_KEYS: SpinLock<alloc::vec::Vec<(usize, usize)>> =
    SpinLock::new(alloc::vec::Vec::new());

/// 占槽之后、撤页之前（真实在途窗口）：这次移交正持着调用者的 pend。
///
/// 探针只做一次已有的 `try_lock` 局部读数，不去取同一把锁，也不跨层回调。
fn pend_held_in_flight(_source: &FlatRange, _page: PageIndex) {
    let task = IN_FLIGHT_TASK.lock().clone().expect("armed in-flight task");
    PEND_HELD_IN_FLIGHT.store(task.pend.try_lock().is_none(), Ordering::Relaxed);
}

/// 每条关系在 `signal` 之前：记下它等待位的键，并看这一刻 pend 是不是已经放了。
fn pend_free_at_wake(space: Asid, view: usize) {
    let task = WAKE_TASK.lock().clone().expect("armed wake task");
    PEND_FREE_AT_WAKE.store(task.pend.try_lock().is_some(), Ordering::Relaxed);
    WOKEN_KEYS.lock().push((space.get(), view));
}

/// 一次 `Transfer`：两个引用句柄 + 各自相对的页号（四字内联，没有描述符）。
fn transfer_call(from: usize, from_page: usize, to: usize, to_page: usize) -> EnvCall {
    EnvCall::Memory(MemoryCall::Transfer {
        from: reference(from),
        from_page,
        to: reference(to),
        to_page,
    })
}

/// 取一份**没物化**的私有来源引用：拿到手的每一页都是 Fresh。
fn acquire_fresh(caller: &Caller, pages: usize) -> isize {
    use env::{AcquireSource, Allocation, PieToken};
    caller.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Allocation(
            Allocation::new(pages * PAGE_SIZE, PAGE_SIZE).expect("allocation"),
        ),
        retention: PieToken::NONE,
    }))
}

/// 一份引用此刻向来源解析到的窗口。
fn window_of(caller: &Caller, handle: usize) -> FlatRange {
    caller
        .task
        .pend
        .references
        .get(handle)
        .expect("hold")
        .resolve()
        .expect("window")
}

/// 窗口里已放下承载的槽：`(相对页号, 物理起址)`；墓碑不在里面。
fn slots_of(range: &FlatRange) -> alloc::vec::Vec<(usize, usize)> {
    range
        .materialized_frames()
        .expect("slot snapshot")
        .into_iter()
        .map(|(offset, frame)| (offset.get(), frame.physical_address()))
        .collect()
}

/// 窗口里第 `page` 页的承载物理起址。
fn slot_at(range: &FlatRange, page: usize) -> usize {
    slots_of(range)
        .into_iter()
        .find(|(offset, _)| *offset == page)
        .expect("a carrier at that page")
        .1
}

/// 窗口里第 `page` 页是不是墓碑（承载已经移出）——与“从没放过”的 Fresh 分开，
/// 且不物化任何东西。
fn tombstone(range: &FlatRange, page: usize) -> bool {
    let one = range
        .range(PageOffset::new(page), PageCount::new(1).expect("one page"))
        .expect("page in window");
    !one.can_materialize(PageOffset::ZERO).expect("page state") && slots_of(&one).is_empty()
}

/// 把一份私有引用的第 `page` 页物化出承载（新取来的来源全是 Fresh）。
fn materialize_page(caller: &Caller, handle: usize, page: usize) {
    let one = window_of(caller, handle)
        .range(PageOffset::new(page), PageCount::new(1).expect("one page"))
        .expect("page in window");
    drop(one.materialize().expect("materialize the page"));
}

/// 别的任务用自己的私有来源铸一根根，并把一枚子 Pie 交给 `me`。
///
/// 交给 `me` 的那一枚 `sire` 非空：它不是 root token，拿它作不了主。
fn grant_to(giver: &Caller, me: &Caller, pages: usize) -> env::PieToken {
    use env::{Mark, Permission, PieToken};
    let source = acquire_fresh(giver, pages);
    assert!(source > 0, "giver source: {source}");
    materialize_page(giver, source as usize, 0);
    let root = crate::entry::call::pie::create_pole(&giver.task, reference(source as usize), true)
        .expect("the giver's root grant");
    assert_eq!(
        giver.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(source as usize)
        })),
        0
    );
    let child = pend::accord(
        &giver.task,
        root,
        &Arc::downgrade(&me.task),
        Permission::FETCH | Permission::STORE | Permission::VEST,
        Mark::NONE,
    )
    .expect("a child grant");
    PieToken::mint(child)
}

/// 真实入口的移交成功面：两端窗口各自解析、页号只换一次、承载与整页内容一起走。
///
/// 覆盖：私有两端同/跨 Flat、非零窗口偏移、目标 Fresh 与 Missing 落槽、源墓碑
/// 各自当场拒，以及根 Grant 接管之后的 Pole 来源。
pub fn transfer_pages() {
    use super::transfer::{fill_pattern, pattern_holds};
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use env::{AcquireSource, MemoryFail, PieToken, ReferenceRange};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    // 源 A 四页全物化（各自一个承载）；目标 B 两页都没物化（Fresh）。
    let (_, _, a) = user.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let b = acquire_fresh(&user, 2);
    assert!(b > 0, "fresh target: {b}");
    let a_range = window_of(&user, a.get());
    let b_range = window_of(&user, b as usize);
    assert_eq!(
        slots_of(&a_range).len(),
        4,
        "the source starts fully resident"
    );
    assert!(slots_of(&b_range).is_empty(), "the target starts fresh");
    let pa: alloc::vec::Vec<usize> = (0..4).map(|page| slot_at(&a_range, page)).collect();
    fill_pattern(PhysAddr::from_raw(pa[0]));

    // 跨 Flat、目标 Fresh。
    let before = frame_occupied() - block_occupied();
    assert_eq!(
        user.call(transfer_call(a.get(), 0, b as usize, 0)),
        0,
        "a resident page moves into a fresh slot"
    );
    assert_eq!(
        frame_occupied() - block_occupied(),
        before,
        "the carrier moved; no frame was allocated or lost"
    );
    assert_eq!(
        slots_of(&b_range),
        alloc::vec![(0, pa[0])],
        "the target seat holds the same physical start"
    );
    assert!(
        pattern_holds(PhysAddr::from_raw(pa[0])),
        "all 4096 bytes came along"
    );
    assert!(
        tombstone(&a_range, 0),
        "the source page becomes a tombstone"
    );
    assert_eq!(
        user.call(transfer_call(a.get(), 0, b as usize, 1)),
        MemoryFail::SourceEmpty.code(),
        "a tombstone has nothing to hand over"
    );

    // 同一份来源里换槽：承载与 PA 不动，原来的槽成墓碑。
    assert_eq!(
        user.call(transfer_call(b as usize, 0, b as usize, 1)),
        0,
        "a page moves inside its own source"
    );
    assert_eq!(
        slots_of(&b_range),
        alloc::vec![(1, pa[0])],
        "the carrier keeps its physical start at the new seat"
    );
    assert!(tombstone(&b_range, 0));

    // 目标墓碑也照收：同一次承载落回 A 的空槽。
    assert_eq!(
        user.call(transfer_call(b as usize, 1, a.get(), 0)),
        0,
        "a tombstone target takes the carrier"
    );
    assert_eq!(
        slots_of(&a_range),
        alloc::vec![(0, pa[0]), (1, pa[1]), (2, pa[2]), (3, pa[3])],
        "the carrier returns and the neighbours stay where they were"
    );
    assert!(tombstone(&b_range, 1));

    // 非零窗口偏移：这段引用只覆盖 A 的第 1..3 页，页号相对**这段窗口**。
    // 只有一次换算才能把第 2 页交出去；叠加两次就会动到第 3 页。
    let sub = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Reference(ReferenceRange::new(a, 1, 2).expect("sub window")),
        retention: PieToken::NONE,
    }));
    assert!(sub > 0, "a sub-window reference: {sub}");
    let sub_range = window_of(&user, sub as usize);
    assert_eq!(
        sub_range.index().get(),
        a_range.index().get() + 1,
        "the sub-window starts at the second page"
    );
    assert_eq!(sub_range.page_count().get(), 2);
    fill_pattern(PhysAddr::from_raw(pa[2]));
    assert_eq!(
        user.call(transfer_call(sub as usize, 1, b as usize, 0)),
        0,
        "the page number is relative to the reference's own window"
    );
    assert_eq!(
        slots_of(&b_range),
        alloc::vec![(0, pa[2])],
        "exactly the second page of that window moved"
    );
    assert!(
        tombstone(&a_range, 2) && !tombstone(&a_range, 1) && !tombstone(&a_range, 3),
        "the neighbouring pages are untouched"
    );

    // 根 Grant 接管之后：整段来源由 Pole 持有，调用者表里那枚 root token 才是管理者。
    // 只物化第一页，第二页留着 Fresh，正好当根阶段的目标槽。
    let root_source = acquire_fresh(&user, 2);
    materialize_page(&user, root_source as usize, 0);
    let root_pa = slot_at(&window_of(&user, root_source as usize), 0);
    fill_pattern(PhysAddr::from_raw(root_pa));
    let root =
        crate::entry::call::pie::create_pole(&user.task, reference(root_source as usize), true)
            .expect("a private whole source becomes a root grant");
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(root_source as usize)
        })),
        0
    );
    let held = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: root,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(
        held > 0,
        "the caller's own root token lends its window: {held}"
    );
    assert_eq!(
        user.call(transfer_call(held as usize, 0, held as usize, 1)),
        0,
        "a root-managed source hands over its page"
    );
    assert_eq!(
        slots_of(&window_of(&user, held as usize)),
        alloc::vec![(1, root_pa)],
        "the carrier lands in the fresh slot"
    );
    assert!(pattern_holds(PhysAddr::from_raw(root_pa)));

    // 公开 `Narrow` 可达的 FETCH-only 根**仍是管理者**：管理判据不看权限位，
    // 只看那枚 root token 自己还活、还没交出。
    let narrow_source = acquire_fresh(&user, 2);
    materialize_page(&user, narrow_source as usize, 0);
    let narrow_pa = slot_at(&window_of(&user, narrow_source as usize), 0);
    let narrow_root =
        crate::entry::call::pie::create_pole(&user.task, reference(narrow_source as usize), true)
            .expect("a private whole source becomes a root grant");
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(narrow_source as usize)
        })),
        0
    );
    assert_eq!(
        pend::reduce(&user.task, narrow_root, Permission::FETCH),
        Ok(()),
        "the public narrow reaches a FETCH-only root"
    );
    let narrow_hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: narrow_root,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(narrow_hold > 0, "a FETCH-only root still lends its window");
    assert_eq!(
        user.call(transfer_call(
            narrow_hold as usize,
            0,
            narrow_hold as usize,
            1
        )),
        0,
        "a FETCH-only root still commands its own source"
    );
    assert_eq!(
        slots_of(&window_of(&user, narrow_hold as usize)),
        alloc::vec![(1, narrow_pa)],
        "the carrier lands in the fresh slot"
    );
    user.team.space.nest.audit();
    for handle in [
        a.get(),
        b as usize,
        sub as usize,
        held as usize,
        narrow_hold as usize,
    ] {
        assert_eq!(
            user.call(EnvCall::Memory(MemoryCall::Release {
                reference: reference(handle)
            })),
            0
        );
    }
}

/// 真实入口的拒绝面：两端管理授权各自把关、失效的授权写点、可重试与永久拒结。
pub fn transfer_refusals() {
    use crate::core::memory::flat::Frame;
    use crate::core::work::mail::nole::NoleMeta;
    use crate::core::work::pend::SourceAuthorization;
    use env::{AcquireSource, Mark, MemoryFail, Permission, PieToken, ReferenceRange};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let other = Caller::new(false);

    // 错误句柄：哨兵号与已经解掉的旧号都不在表里。
    assert_eq!(
        user.call(transfer_call(0, 0, 0, 0)),
        MemoryFail::Denied.code(),
        "the sentinel handle is nobody's reference"
    );
    let (_, _, released) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: released
        })),
        0
    );
    assert_eq!(
        user.call(transfer_call(released.get(), 0, released.get(), 1)),
        MemoryFail::Denied.code(),
        "a released handle is gone from the table"
    );

    // 目标占位与窗口越界：两端本来就合法的调用，各自的码分开。
    let (_, _, pair) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    assert_eq!(
        user.call(transfer_call(pair.get(), 0, pair.get(), 1)),
        MemoryFail::DestinationOccupied.code(),
        "a target seat that holds a carrier refuses the move"
    );
    assert_eq!(
        user.call(transfer_call(pair.get(), 0, pair.get(), 0)),
        MemoryFail::DestinationOccupied.code(),
        "the same seat on both ends is not a move"
    );
    assert_eq!(
        user.call(transfer_call(pair.get(), 2, pair.get(), 1)),
        MemoryFail::NoRegion.code(),
        "a page past the reference window is out of range"
    );

    // 私有阶段只认真正的属主。夹具直造一份属主是**别人**的私有来源引用：
    // 当前用户入口造不出这种引用（私有引用只生在属主自己的表里），这里把它放进本表，
    // 管理判据不能被“表里有这一格”冒充。
    let foreign = crate::core::memory::flat::Allocation::new(2, PAGE_SIZE).expect("allocation");
    let foreign = Flat::acquire(foreign).expect("flat");
    let foreign_range = foreign.whole();
    drop(
        foreign_range
            .range(PageOffset::ZERO, PageCount::new(1).expect("one page"))
            .expect("first page")
            .materialize()
            .expect("materialize the first page"),
    );
    let authorization = SourceAuthorization::new(foreign_range, Access::all(), other.task.ident.id)
        .expect("authorization");
    let foreign_handle = user
        .task
        .pend
        .references
        .insert(
            authorization,
            PageOffset::ZERO,
            PageCount::new(2).expect("two pages"),
            Access::all(),
            None,
            None,
        )
        .expect("insert");
    assert_eq!(
        user.call(transfer_call(foreign_handle, 0, foreign_handle, 1)),
        MemoryFail::Denied.code(),
        "the private owner, not the reference table, decides who may hand over"
    );

    // 别人铸的根交给本任务的那一枚是**子 Pie**（sire 非空）：它既不能当源端，
    // 也不能当目标端——两端各管各的。
    let mine = acquire_fresh(&user, 2);
    let child = grant_to(&other, &user, 2);
    let child_hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: child,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(
        child_hold > 0,
        "a child grant yields a reference: {child_hold}"
    );
    assert_eq!(
        user.call(transfer_call(child_hold as usize, 0, mine as usize, 0)),
        MemoryFail::Denied.code(),
        "the source end is checked on its own"
    );
    assert_eq!(
        user.call(transfer_call(mine as usize, 0, child_hold as usize, 0)),
        MemoryFail::Denied.code(),
        "the target end is checked on its own"
    );

    // 已撤销：闸门的短读一读就是结论。
    let (_, _, revocable) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    user.task
        .pend
        .references
        .get(revocable.get())
        .expect("hold")
        .source()
        .revoke()
        .expect("nothing in flight");
    assert_eq!(
        user.call(transfer_call(revocable.get(), 0, revocable.get(), 1)),
        MemoryFail::Denied.code(),
        "a revoked source hands nothing over"
    );

    // 根已经交出去（ONLY 那一枚 Accord 给别的任务）：heir 非空，这一枚自己不再可用；
    // 子 Pie 退场把 heir 收回之后，同一次移交又过得了管理判据。
    let exclusive = acquire_fresh(&user, 2);
    materialize_page(&user, exclusive as usize, 0);
    let only =
        crate::entry::call::pie::create_pole(&user.task, reference(exclusive as usize), false)
            .expect("an exclusive root");
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(exclusive as usize)
        })),
        0
    );
    let exclusive_hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: only,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(exclusive_hold > 0, "the exclusive root lends its window");
    assert!(
        pend::accord(
            &user.task,
            only,
            &Arc::downgrade(&other.task),
            Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
            Mark::NONE,
        )
        .is_ok(),
        "the root hands its exclusive grant over"
    );
    assert_eq!(
        user.call(transfer_call(
            exclusive_hold as usize,
            0,
            exclusive_hold as usize,
            1
        )),
        MemoryFail::Denied.code(),
        "a root already handed over is not a manager"
    );

    // 已 seal 的 Pole 不再是管理者：判据看的是 `pie.alive()`——它的 Meta 那一半
    // （不只 Grant）——而不是“根 Grant 还能答题”。公开 `Seal` 做的正是这一下：
    // `mail.seal()` 之后那一枚仍在表里，而它的 Grant 还没失效。
    //
    // 这里用**保留那一份**作两端：它自己强持范围，`Hold::resolve()` 不再向 Pole
    // 解析，所以这条拒绝只可能来自管理判据里 Meta 那一半。
    let sealed_source = acquire_fresh(&user, 2);
    materialize_page(&user, sealed_source as usize, 0);
    let sealed_root =
        crate::entry::call::pie::create_pole(&user.task, reference(sealed_source as usize), true)
            .expect("a private whole source becomes a root grant");
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(sealed_source as usize)
        })),
        0
    );
    let sealed_hold = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: sealed_root,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(sealed_hold > 0, "the root token lends its window");
    // 这一枚保留授权同样是内核铸、引导发放的 cap，用户入口拿不到。
    let cap = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::mint_retention(user.task.ident.id).expect("retention cap")),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        user.task.pend.pies.lock().push(pie);
        token
    };
    let sealed_retained = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Reference(
            ReferenceRange::new(reference(sealed_hold as usize), 0, 2).expect("window"),
        ),
        retention: cap,
    }));
    assert!(sealed_retained > 0, "a retained window: {sealed_retained}");
    let sealed_meta = pend::locate(&user.task, sealed_root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    // 只调用失活步骤；不覆盖入口 Seal 的 pend 与 close_maps 编排。
    crate::core::work::mail::pole::seal(&sealed_meta);
    assert!(!sealed_meta.alive(), "the seal lands on the Pole itself");
    assert!(
        sealed_meta
            .grant(sealed_root)
            .is_some_and(|grant| grant.permission().is_some()),
        "the root Grant itself still answers after the seal"
    );
    assert_eq!(
        user.call(transfer_call(
            sealed_retained as usize,
            0,
            sealed_retained as usize,
            1
        )),
        MemoryFail::Denied.code(),
        "a sealed Pole is not a manager even though its Grant still answers"
    );

    // 任务走到尽头：先清引用，再关控制与失活；旧句柄在同一张表里当场查不到。
    {
        let _commit = crate::core::work::commit();
        other.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    }
    pend::doom(&other.task);
    assert_eq!(
        user.call(transfer_call(
            exclusive_hold as usize,
            0,
            exclusive_hold as usize,
            1
        )),
        0,
        "with the hand-over withdrawn the root commands again"
    );
    assert_eq!(
        user.call(transfer_call(child_hold as usize, 0, mine as usize, 0)),
        MemoryFail::Denied.code(),
        "the other task's references are gone with it"
    );

    // 独立保留压着源页：那一页正被保留，不是空闲的承载（可重试，不是拒结）。
    let (_, _, held_source) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    // 这枚保留授权是**生产真形态**（内核铸），但当前由引导资源发放、
    // 用户入口拿不到——夹具直铸一份，才能走到下面那条判据。
    let cap = {
        let pie = pend::Pie::try_new(
            Mail::Nole(NoleMeta::mint_retention(user.task.ident.id).expect("retention cap")),
            Mark::NONE,
            Permission::FETCH | Permission::STORE | Permission::VEST,
            None,
        )
        .expect("pie");
        let token = pie.token();
        user.task.pend.pies.lock().push(pie);
        token
    };
    let retained = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Reference(ReferenceRange::new(held_source, 0, 2).expect("window")),
        retention: cap,
    }));
    assert!(retained > 0, "a retained sub-window: {retained}");
    assert_eq!(
        user.call(transfer_call(held_source.get(), 0, held_source.get(), 1)),
        MemoryFail::Busy.code(),
        "a residency over the source page is not an idle carrier"
    );

    // 借用来源（设备区）：承载在别处，整份来源永久不可移交——不是权限问题。
    // 这也是生产真形态，但由引导设备/`pole::region` 接线建立；当前用户入口只能
    // 从**自己的私有来源** `create_pole`，铸不出借用来源的根。
    let frame = Frame::allocate().expect("frame");
    let meta = pole::region(frame.physical_address(), 2 * PAGE_SIZE, user.task.ident.id)
        .expect("borrowed region");
    let pie = pend::Pie::try_new(
        Mail::Pole(meta.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )
    .expect("pie");
    let token = pie.token();
    user.task.pend.pies.lock().push(pie);
    let authorization = SourceAuthorization::new(meta.source(), Access::all(), user.task.ident.id)
        .expect("authority");
    meta.attach_source(&authorization);
    assert!(
        authorization
            .adopt(&meta, token, meta.grant(token).expect("grant"))
            .is_ok()
    );
    let borrowed = user.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token,
            offset: 0,
            count: 2,
        },
        retention: PieToken::NONE,
    }));
    assert!(borrowed > 0, "a borrowed window: {borrowed}");
    assert_eq!(
        user.call(transfer_call(borrowed as usize, 0, borrowed as usize, 1)),
        MemoryFail::NotTransferable.code(),
        "a borrowed source is refused for good, not as a permission failure"
    );
    assert_ne!(
        MemoryFail::NotTransferable.code(),
        MemoryFail::Denied.code()
    );

    // 调用者自己走到尽头：引用表先清空，旧句柄当场作废。
    let doomed = Caller::new(false);
    let (_, _, stale) = doomed.attach_alloc_reference(2 * PAGE_SIZE, 6);
    {
        let _commit = crate::core::work::commit();
        doomed.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    }
    pend::doom(&doomed.task);
    assert_eq!(
        doomed.call(transfer_call(stale.get(), 0, stale.get(), 1)),
        MemoryFail::Denied.code(),
        "a doomed caller has no reference left to hand over"
    );
}

/// 清单的唯一消费者：真实入口里“执行时 pend 在场、放行时 pend 已放”。
///
/// 这里只证明这份清单**到了消费者**、键就是压着源页那段关系的等待位，
/// 且放行发生在 pend 之外；真实的 Room 等待链（block → signal → resume）
/// 不在本夹具的证明范围。
pub fn transfer_wakes_relations() {
    use crate::core::memory::nest::identity::fail_next_shootdown;
    use env::MemoryFail;

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let user = Caller::new(false);
    let asid = user.team.space.nest.asid();
    *IN_FLIGHT_TASK.lock() = Some(user.task.clone());
    *WAKE_TASK.lock() = Some(user.task.clone());
    crate::core::memory::transfer::arm_in_flight(Some(pend_held_in_flight));
    crate::entry::call::memory::arm_wake_probe(Some(pend_free_at_wake));

    let reset = || {
        PEND_HELD_IN_FLIGHT.store(false, Ordering::Relaxed);
        PEND_FREE_AT_WAKE.store(false, Ordering::Relaxed);
        WOKEN_KEYS.lock().clear();
    };

    // 源页上真压着一条关系（已发布 PTE）：它必须出现在清单里。
    let (view, _, source) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let target = acquire_fresh(&user, 2);
    reset();
    assert_eq!(
        user.call(transfer_call(source.get(), 0, target as usize, 0)),
        0,
        "the page moves"
    );
    assert!(
        PEND_HELD_IN_FLIGHT.load(Ordering::Relaxed),
        "the caller's pend is held while the transfer executes"
    );
    assert!(
        PEND_FREE_AT_WAKE.load(Ordering::Relaxed),
        "the wake happens after the pend is released"
    );
    assert_eq!(
        WOKEN_KEYS.lock().clone(),
        alloc::vec![(asid.get(), view)],
        "exactly the relation pressing the source page reaches the consumer"
    );

    // 失败回退：清单照旧到消费者手里——不因为结果是 Err 就把通知吞掉。
    let (failing_view, _, failing_source) = user.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let failing_target = acquire_fresh(&user, 1);
    reset();
    fail_next_shootdown(asid);
    assert_eq!(
        user.call(transfer_call(
            failing_source.get(),
            0,
            failing_target as usize,
            0
        )),
        MemoryFail::Busy.code(),
        "an unconfirmed sync is retryable"
    );
    assert_eq!(
        WOKEN_KEYS.lock().clone(),
        alloc::vec![(asid.get(), failing_view)],
        "a failed move still names the relations it touched"
    );
    assert!(PEND_FREE_AT_WAKE.load(Ordering::Relaxed));
    // 两端按原态还回：源页又都是承载，目标那一格仍空。
    assert_eq!(
        slots_of(&window_of(&user, failing_source.get())).len(),
        2,
        "the source keeps both carriers after the rollback"
    );
    assert!(
        slots_of(&window_of(&user, failing_target as usize)).is_empty(),
        "the target stays fresh after the rollback"
    );
    user.team
        .space
        .nest
        .retry_invalidation()
        .expect("settle the injected failure");
    user.team.space.nest.audit();

    crate::entry::call::memory::arm_wake_probe(None);
    crate::core::memory::transfer::arm_in_flight(None);
    *IN_FLIGHT_TASK.lock() = None;
    *WAKE_TASK.lock() = None;
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: failing_source
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(target as usize)
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(failing_target as usize)
        })),
        0
    );
}

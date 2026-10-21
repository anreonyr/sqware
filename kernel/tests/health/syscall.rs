#![cfg(debug_assertions)]

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use env::{
    ControlCall, DebugCall, EnvCall, MemoryCall, PieCall, ProgramKind, RoomCall, UnitCall, VirtAddr,
};

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PageCount, PageIndex, PageOffset, PhysAddr, VirtAddr as KVirt};
use crate::core::memory::flat::{Flat, FlatRange};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{Access, Asid, NestError, Placement, RegionId, View};
use crate::core::work::mail::{Mail, hole, pole};
use crate::core::work::pend::{self, Need, Permission};
use crate::core::work::unit::space::Space;
use crate::core::work::unit::space::SpaceBuilder;

use crate::core::work::room::conductor::tests::with_running;

use crate::core::work::unit::task::Task;
use crate::core::work::unit::team::{Team, TeamBuilder};
use crate::entry::call as envcall;
use crate::entry::report::backtrace::StackReader;
use crate::machine::mode;
use crate::machine::switcher::context::{Gprs, TrapContext};
use crate::machine::sync::SpinLock;

#[path = "retention/mod.rs"]
mod external_retention;
pub use external_retention::{accept as external_retention, delegated as retention_delegation};

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
            EnvCall::Pie(call) => (call.slot(), call.pack()),
            EnvCall::Hole(call) => (call.slot(), call.pack()),
            EnvCall::Pole(call) => (call.slot(), call.pack()),
            EnvCall::Nole(call) => (call.slot(), call.pack()),
            EnvCall::Tole(call) => (call.slot(), call.pack()),
            _ => unreachable!(),
        };
        self.raw(slot, regs)
    }

    /// 双值返回那一格（`Attach`）：读 `a0` / `a1`。
    fn pair(&self, call: EnvCall) -> (isize, usize) {
        let (slot, regs) = match call {
            EnvCall::Memory(call) => (call.slot(), call.pack()),
            EnvCall::Pole(call) => (call.slot(), call.pack()),
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

pub fn creator_and_owner() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    let creator = Caller::new(false);
    let mut registry = crate::entry::boot::resources::Registry::default();
    crate::entry::call::resources::register(&mut registry).unwrap();
    registry.freeze().unwrap().grant(&creator.task).unwrap();
    let owner_team = TeamBuilder::new(SpaceBuilder::user().build().unwrap())
        .sire(crate::core::work::unit::task::TaskWeak::stored(
            Arc::downgrade(&creator.task),
            crate::core::work::unit::copies::Origin::Sire,
        ))
        .spawn().unwrap();
    let owner = owner_team.task().hold().unwrap();
    let id = creator.call(EnvCall::Unit(UnitCall::Build {
        kind: ProgramKind::User,
        sire: owner.ident.id,
    }));
    assert!(id > 0);
    let team = owner.heir(env::TeamId::new(id as usize)).unwrap();
    assert_eq!(team.creator(), creator.task.ident.id);
    assert_eq!(team.sire(), Some(owner.ident.id));
    assert_ne!(creator.task.ident.id, owner.ident.id);
    team.space.nest.attach_region(RegionId::Normal, 0x4000_0000, mode::upper().as_usize()).unwrap();
    let reservation = super::support::reserve(&team.space, KVirt::wrap(0x4000_0000), PAGE_SIZE).unwrap();
    let code = super::support::attach_frames(
        &team.space,
        reservation,
        alloc::vec![crate::tag!(Pole, crate::core::memory::flat::Frame::allocate().unwrap())],
        Access::READ | Access::EXECUTE,
        Access::READ | Access::EXECUTE,
    ).unwrap();
    let task = creator.call(EnvCall::Unit(UnitCall::Spawn {
        team: team.id,
        entry: 0x4000_0000,
        args: VirtAddr::new(SCRATCH_AT),
        count: 0,
        stack: 0,
    }));
    assert!(task > 0, "first task spawn: {task}");
    let child = Caller {
        task: crate::core::work::room::scheduler::roster::muster(env::TaskId::new(task as usize))
            .and_then(|task| task.upgrade()).unwrap(),
        team: team.clone(),
        scratch: core::cell::RefCell::new(None),
        mapped: core::cell::RefCell::new(alloc::vec::Vec::new()),
    };
    assert_eq!(child.call(EnvCall::Unit(UnitCall::Creator)), creator.task.ident.id.get() as isize);
    assert_eq!(child.call(EnvCall::Unit(UnitCall::Sire)), owner.ident.id.get() as isize);
    assert_eq!(child.call(EnvCall::Unit(UnitCall::Build {
        kind: ProgramKind::User,
        sire: child.task.ident.id,
    })), env::UnitFail::Denied.code());
    crate::core::work::room::messenger::slay(&child.task);
    assert!(child.task.reaped());
    drop(child);
    team.space.nest.dispose(code).unwrap();
    assert_eq!(creator.call(EnvCall::Unit(UnitCall::Oust { team: team.id })), 0);
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

    let delegated = {
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
            retention: delegated,
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        0,
        "a kernel-minted cap held in this task's own account authorizes"
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Traverse {
            retention: PieToken::mint(0xdead_beef),
            range: VirtAddr::new(scratch),
            out: VirtAddr::new(scratch + 64),
        })),
        MemoryFail::Denied.code(),
        "a token this task does not hold authorizes nothing"
    );
    assert_eq!(pend::release(&user.task, delegated).unwrap(), 1);

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
        crate::entry::call::pole::create_pole(task, reference(data as usize), true),
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
    let root = crate::entry::call::pole::create_pole(&owner.task, source, true).unwrap();
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
    left.team.space.nest.release_reservation(pending).unwrap();
    // 结清只有一条规则：登记与源侧成员一起撤。
    left.team.space.retire_view(id);
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
    assert_eq!(
        entry.control.as_ref().unwrap().control().0,
        user.task.ident.id
    );

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
    for entry in owned {
        user.team
            .space
            .nest
            .dispose(entry.view().expect("published").clone())
            .expect("dispose");
        user.team.space.retire_view(entry.binding.id());
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

pub fn resource_domains() {
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use crate::core::memory::nest::table::TABLE_LIVE;
    use crate::core::work::room::{messenger, scheduler};
    use env::{
        HoleCall, HoleCondition, NoleCall, PoleCall, PoleCondition, ToleAwaitReply, ToleCall,
        ToleEventFail, ToleEventSource, Wait,
    };

    scheduler::init().unwrap();
    let seed = Asid::try_allocate().unwrap();
    Asid::release_unpublished(seed);
    let sites = messenger::site_count();
    let tables = TABLE_LIVE.load(Ordering::Relaxed);
    let frames = frame_occupied() - block_occupied();
    let caller = Caller::new(false);
    with_running(caller.task.clone(), || {
        let raw = caller.call(EnvCall::Hole(HoleCall::Unseal {
            mark: env::Mark::of("typed-hole"),
            limits: env::HoleLimits::default(),
        }));
        assert!(raw > 0);
        let hole_token = env::PieToken::mint(raw as usize);
        let raw = caller.call(EnvCall::Nole(NoleCall::Unseal));
        assert!(raw > 0);
        let nole_token = env::PieToken::mint(raw as usize);
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Ring { token: nole_token })),
            env::HoleFail::Denied.code()
        );
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Ring { token: hole_token })),
            env::NoleFail::Denied.code()
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Seal { token: nole_token })),
            env::HoleFail::Denied.code()
        );
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Ring { token: nole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Wait {
                token: nole_token,
                millis: Wait::POLL,
            })),
            1
        );
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Hush { token: nole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Wait {
                token: nole_token,
                millis: Wait::POLL,
            })),
            0
        );

        let scratch = caller.scratch_page();
        caller.write_words(scratch, &[0x1234]);
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Push {
                token: hole_token,
                msg: VirtAddr::new(scratch),
                len: 8,
            })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            0,
            "queued data is not a counted notification"
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Peek { token: hole_token })),
            8
        );
        let frame = unsafe { &*(caller.task.ident.frame_pa.as_usize() as *const TrapContext) };
        assert_eq!(frame.gpr.x(Gprs::A1), caller.task.ident.id.get());
        assert_eq!(frame.gpr.x(Gprs::A2), 1);
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Pull {
                token: hole_token,
                buf: VirtAddr::new(scratch + 16),
                max: 8,
                oversize: env::Oversize::Keep,
            })),
            8
        );
        let frame = unsafe { &*(caller.task.ident.frame_pa.as_usize() as *const TrapContext) };
        assert_eq!(frame.gpr.x(Gprs::A1), caller.task.ident.id.get());
        assert_eq!(frame.gpr.x(Gprs::A2), 0);
        let mut bytes = [0u8; 8];
        assert!(
            caller
                .team
                .space
                .copy_in(&mut bytes, KVirt::wrap(scratch + 16))
        );
        assert_eq!(usize::from_ne_bytes(bytes), 0x1234);

        let raw = caller.call(EnvCall::Tole(ToleCall::Unseal { shared: true }));
        assert!(raw > 0);
        let tole_token = env::PieToken::mint(raw as usize);
        let event = ToleEventSource::Hole {
            token: hole_token,
            condition: HoleCondition::Signal,
        };
        assert_eq!(
            caller.call(EnvCall::Tole(ToleCall::Seal { token: hole_token })),
            env::ToleFail::Denied.code()
        );
        assert_eq!(
            caller.call(EnvCall::Tole(ToleCall::Attach {
                tole: tole_token,
                source: event,
            })),
            0
        );
        let await_event = || {
            let tag = caller.call(EnvCall::Tole(ToleCall::Await {
                tole: tole_token,
                millis: Wait::POLL,
            }));
            assert!(tag >= 0);
            let frame = unsafe { &*(caller.task.ident.frame_pa.as_usize() as *const TrapContext) };
            ToleAwaitReply::of([tag as usize, frame.gpr.x(Gprs::A1), frame.gpr.x(Gprs::A2)])
                .unwrap()
        };
        assert_eq!(await_event(), ToleAwaitReply::Pending);
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Ring { token: hole_token })),
            0
        );
        assert_eq!(
            await_event(),
            ToleAwaitReply::Source {
                source: event,
                fail: None,
            }
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            1
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Hush { token: hole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            0
        );

        let meta = pend::locate(&caller.task, hole_token)
            .unwrap()
            .mail()
            .hole()
            .unwrap();
        let key = hole::key(&meta, HoleCondition::Signal);
        let life = meta.life();
        let watcher = 0xf00d;
        messenger::forward(key, life.clone(), watcher, life.clone()).unwrap();
        for _ in 0..2 {
            assert_eq!(
                caller.call(EnvCall::Hole(HoleCall::Ring { token: hole_token })),
                0
            );
        }
        assert!(matches!(
            messenger::wait::<env::RoomFail>(
                messenger::WakeKey::Tole { id: watcher },
                life.clone(),
                core::time::Duration::MAX,
            ),
            Ok(messenger::Handoff::Resume(()))
        ));
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Hush { token: hole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            1,
            "one acknowledgement must not erase the second event"
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Hush { token: hole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            0
        );

        let handle = caller.call(EnvCall::Memory(MemoryCall::Acquire {
            source: env::AcquireSource::Allocation(
                env::Allocation::new(PAGE_SIZE, PAGE_SIZE).unwrap(),
            ),
            retention: env::PieToken::NONE,
        }));
        assert!(handle > 0);
        let source = reference(handle as usize);
        let raw = caller.call(EnvCall::Pole(PoleCall::Unseal {
            reference: source,
            shared: true,
        }));
        assert!(raw > 0);
        let pole_token = env::PieToken::mint(raw as usize);
        assert_eq!(
            caller.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
            0
        );
        let (address, size) = caller.pair(EnvCall::Pole(PoleCall::Open { token: pole_token }));
        assert!(address > 0);
        assert_eq!(size, PAGE_SIZE);
        let bit = env::Bit::of(2).unwrap();
        assert_eq!(
            caller.call(EnvCall::Pole(PoleCall::Ring {
                token: pole_token,
                bit
            })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Pole(PoleCall::Wait {
                token: pole_token,
                condition: PoleCondition::Signal(bit),
                millis: Wait::POLL,
            })),
            1
        );
        assert_eq!(
            caller.call(EnvCall::Pole(PoleCall::Hush {
                token: pole_token,
                bit
            })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Pole(PoleCall::Shut { token: pole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Pole(PoleCall::Seal { token: pole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Seal { token: hole_token })),
            0
        );
        assert_eq!(
            await_event(),
            ToleAwaitReply::Source {
                source: event,
                fail: Some(ToleEventFail::Dead),
            }
        );
        assert_eq!(
            caller.call(EnvCall::Tole(ToleCall::Detach {
                tole: tole_token,
                source: event,
            })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Tole(ToleCall::Seal { token: tole_token })),
            0
        );
        assert_eq!(
            caller.call(EnvCall::Tole(ToleCall::Await {
                tole: tole_token,
                millis: Wait::POLL,
            })),
            env::ToleFail::Dead.code()
        );
        assert!(matches!(
            messenger::wait::<env::RoomFail>(
                messenger::WakeKey::Tole { id: watcher },
                life.clone(),
                core::time::Duration::MAX,
            ),
            Ok(messenger::Handoff::Resume(()))
        ));
        assert_eq!(
            caller.call(EnvCall::Hole(HoleCall::Wait {
                token: hole_token,
                condition: HoleCondition::Signal,
                millis: Wait::POLL,
            })),
            env::HoleFail::Dead.code()
        );
        messenger::unforward(key, watcher);
        assert_eq!(
            caller.call(EnvCall::Nole(NoleCall::Seal { token: nole_token })),
            0
        );
        for token in [hole_token, nole_token, pole_token, tole_token] {
            assert!(!pend::locate(&caller.task, token).unwrap().alive());
            pend::release(&caller.task, token).unwrap();
        }
    });
    drop(caller);
    scheduler::roster::prune_dead();
    assert_eq!(messenger::site_count(), sites);
    assert_eq!(TABLE_LIVE.load(Ordering::Relaxed), tables);
    assert_eq!(frame_occupied() - block_occupied(), frames);
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
    let token = crate::entry::call::pole::create_pole(&user.task, reference, true).unwrap();
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
/// 带窗口的分支：精确子窗口只能**单调收窄**，且 acquire / open / 派生 / 撤销都守它。
///
/// 派生那一格现在会交出一枚**独立子资源**：它正好是分支窗口那一段，不是整段来源。
///
/// 窗口用**相对来源起点的绝对页偏移**说（不是相对父分支的偏移），故嵌套派生不会叠加两层。
/// 每一段都走真实调用面（`EnvCall::Pie` / `EnvCall::Memory`），不是直接调内部函数。
pub fn branch_window() {
    use crate::entry::call::pole::create_pole;
    use env::{AcquireSource, GrantRange, MemoryFail, PieCall, PieFail, PieToken, PoleCall};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let owner = Caller::new(false);
    let peer = Caller::new(false);
    let subset = Permission::FETCH | Permission::STORE | Permission::VEST;
    let dst = peer.task.ident.id;

    let token = |code: isize| {
        assert!(code > 0, "token: {code}");
        PieToken::mint(code as usize)
    };
    let derive = |caller: &Caller, src: PieToken, offset: usize, count: usize| {
        pie_call(
            caller,
            PieCall::Derive {
                src,
                subset,
                mark: env::Mark::NONE,
                range: GrantRange::Pages { offset, count },
            },
        )
    };
    let accord = |caller: &Caller, src: PieToken, dst: env::TaskId| {
        pie_call(
            caller,
            PieCall::Accord {
                src,
                dst,
                subset,
                mark: env::Mark::NONE,
            },
        )
    };
    // 一份 4 页私有来源 → 根（整段，无窗口）。
    let (_, _, source_reference) = owner.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let root = token(pole_call(
        &owner,
        PoleCall::Unseal {
            reference: source_reference,
            shared: true,
        },
    ));
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release {
            reference: source_reference
        })),
        0
    );
    assert_eq!(
        pend::locate(&owner.task, root)
            .unwrap()
            .mail()
            .pole()
            .unwrap()
            .source()
            .page_count()
            .get(),
        4,
        "the root covers the whole source"
    );
    assert_eq!(
        pend::locate(&owner.task, root).unwrap().grant().window(),
        None,
        "a plain root carries no window"
    );

    // 越窗 / 空长 / 溢出：都拒，且两张表都不落地。
    for (offset, count, expected) in [
        (2usize, 3usize, PieFail::Denied.code()),
        (1, 0, env::DispatchFail::Unknown.code()),
        (usize::MAX, 2, env::DispatchFail::Unknown.code()),
        (4, 1, PieFail::Denied.code()),
    ] {
        let owner_before = owner.task.pend.pies.lock().len();
        let peer_before = peer.task.pend.pies.lock().len();
        let code = derive(&owner, root, offset, count);
        assert_eq!(
            code, expected,
            "window ({offset},{count}) must be refused"
        );
        assert_eq!(owner.task.pend.pies.lock().len(), owner_before);
        assert_eq!(peer.task.pend.pies.lock().len(), peer_before);
    }

    // 本地派生 (1,2)，再普通 Accord 给对端：范围与权限都由父窗口继承。
    let local = token(derive(&owner, root, 1, 2));
    assert_eq!(
        pend::locate(&owner.task, local).unwrap().grant().window(),
        Some((PageOffset::new(1), PageCount::new(2).unwrap()))
    );
    let child = token(accord(&owner, local, dst));
    assert_eq!(
        pend::locate(&peer.task, child).unwrap().grant().window(),
        Some((PageOffset::new(1), PageCount::new(2).unwrap()))
    );

    // 嵌套：从这一枚再派生只能更窄，不能拿回窗口外的那几页。
    for (offset, count) in [(0usize, 2usize), (2, 2), (1, 3)] {
        let code = derive(&peer, child, offset, count);
        assert_eq!(
            code,
            PieFail::Denied.code(),
            "nested window ({offset},{count}) must not widen"
        );
    }
    let nested_local = token(derive(&peer, child, 1, 1));
    let grand = token(accord(&peer, nested_local, owner.task.ident.id));
    assert_eq!(
        pend::locate(&owner.task, grand).unwrap().grant().window(),
        Some((PageOffset::new(1), PageCount::new(1).unwrap()))
    );

    // Open 只映窗口那一段：长度是它，PA 次序与来源第 1 页对得上。
    let (whole, whole_size) = open_pair(&owner, root);
    assert!(whole > 0, "open root: {whole}");
    assert_eq!(whole_size, 4 * PAGE_SIZE);
    let source_page_1 = owner
        .team
        .space
        .nest
        .physical_at(KVirt::wrap(whole as usize + PAGE_SIZE))
        .unwrap();
    let (windowed, window_size) = open_pair(&peer, child);
    assert!(windowed > 0, "open windowed: {windowed}");
    assert_eq!(
        window_size,
        2 * PAGE_SIZE,
        "a windowed token opens only its window"
    );
    assert_eq!(
        peer.team
            .space
            .nest
            .physical_at(KVirt::wrap(windowed as usize)),
        Some(source_page_1),
        "the window starts at the source's first page in it"
    );
    assert!(
        !peer.team.space.validate_access(
            KVirt::wrap(windowed as usize + 2 * PAGE_SIZE),
            PAGE_SIZE,
            Access::READ
        ),
        "the page after the window is not mapped"
    );
    assert!(
        !peer.team.space.validate_access(
            KVirt::wrap(windowed as usize),
            PAGE_SIZE,
            Access::EXECUTE
        ),
        "a windowed device window is never mapped executable"
    );

    // Acquire 也守窗口：窗口外取不到，窗口内取得到。
    let acquire = |offset: usize, count: usize| {
        peer.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token: child,
                offset,
                count,
            },
            retention: PieToken::NONE,
        }))
    };
    assert_eq!(acquire(0, 1), MemoryFail::Denied.code() as isize);
    assert_eq!(acquire(1, 3), MemoryFail::Denied.code() as isize);
    assert_eq!(acquire(2, 2), MemoryFail::Denied.code() as isize);
    let inside = acquire(1, 2);
    assert!(inside > 0, "acquire inside the window: {inside}");

    // 带窗口的分支交出的是一枚**独立子资源**：范围正好是窗口那一段，不是整段来源。
    let derived = create_pole(&peer.task, reference(inside as usize), true)
        .expect("a windowed branch derives a sub-resource");
    let root_meta = pend::locate(&owner.task, root)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    let window_range = root_meta
        .source()
        .range(PageOffset::new(1), PageCount::new(2).unwrap())
        .unwrap();
    let derived_meta = pend::locate(&peer.task, derived)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    assert!(
        derived_meta.source().same_range(&window_range),
        "the sub-resource is exactly the branch's window on the parent source"
    );

    // 撤销令这一枚与它的 Views 一起失效（窗口不另开一条命）。
    assert_eq!(pie_call(&owner, PieCall::Revoke { dst, token: child }), 0);
    assert!(pend::locate(&peer.task, child).is_none());
    assert!(pend::locate(&owner.task, grand).is_none());
    assert!(
        peer.team
            .space
            .nest
            .translate(KVirt::wrap(windowed as usize))
            .is_none(),
        "revoking the branch closes the view it opened"
    );
    assert_eq!(open_pair(&peer, child).0, env::PoleFail::Denied.code());
    assert!(pend::locate(&owner.task, local).is_some());
    assert_eq!(pie_call(&owner, PieCall::Revoke { dst: owner.task.ident.id, token: local }), 0);
    assert!(pend::locate(&owner.task, local).is_none());

    // 普通 Accord 不带窗口：继承父的范围，这里是整段。
    let (_, _, plain) = owner.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let other = token(pole_call(
        &owner,
        PoleCall::Unseal {
            reference: plain,
            shared: true,
        },
    ));
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release { reference: plain })),
        0
    );
    let plain_remote = token(accord(&owner, other, dst));
    assert_eq!(
        pend::locate(&peer.task, plain_remote)
            .unwrap()
            .grant()
            .window(),
        None,
        "a plain accord still hands the whole source"
    );
    assert_eq!(open_pair(&peer, plain_remote).1, 4 * PAGE_SIZE);

    // 各来源独立关闭自己的分支，不影响另一份来源。
    assert_eq!(pie_call(&owner, PieCall::Release { token: root, mode: env::ReleaseMode::Revoke }), 0);
    assert!(pend::locate(&owner.task, root).is_none());
    assert!(pend::locate(&peer.task, plain_remote).is_some());
    assert_eq!(pie_call(&owner, PieCall::Release { token: other, mode: env::ReleaseMode::Revoke }), 0);
    assert!(pend::locate(&owner.task, other).is_none());
    assert!(pend::locate(&peer.task, plain_remote).is_none());
}

/// 继承那一条路也要**承重**：带窗口的父派生出的子，有效窗口必须还是父那一段。
///
/// 这一条原先没有判据——把 `effective_window` 的“不请求就继承”改成 `Ok(None)`（于是普通
/// Accord 把整段来源交出去）全量测试照绿。故这里三样都量：子枚的 `window()`、
/// `Open` 的长度与首 PA、窗外 `Acquire` 仍拒；再经一层 Accord 与一层收窄之后仍不放大，
/// 继承来的那一枚派生出的子资源也正好是那段窗口。
pub fn window_inheritance() {
    use crate::entry::call::pole::create_pole;
    use env::{AcquireSource, GrantRange, MemoryFail, PieCall, PieToken, PoleCall};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let owner = Caller::new(false);
    let peer = Caller::new(false);
    let subset = Permission::FETCH | Permission::STORE | Permission::VEST;
    let dst = peer.task.ident.id;

    let token = |code: isize| {
        assert!(code > 0, "token: {code}");
        PieToken::mint(code as usize)
    };
    let derive = |caller: &Caller, src: PieToken, offset: usize, count: usize| {
        pie_call(
            caller,
            PieCall::Derive {
                src,
                subset,
                mark: env::Mark::NONE,
                range: GrantRange::Pages { offset, count },
            },
        )
    };
    let accord = |caller: &Caller, src: PieToken, dst: env::TaskId| {
        pie_call(
            caller,
            PieCall::Accord {
                src,
                dst,
                subset,
                mark: env::Mark::NONE,
            },
        )
    };
    let (_, _, source_reference) = owner.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let root = token(pole_call(
        &owner,
        PoleCall::Unseal {
            reference: source_reference,
            shared: true,
        },
    ));
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release {
            reference: source_reference
        })),
        0
    );

    // 一段 (1,2) 的本地派生根，再普通 Accord 给对端；接下来每次都继承父窗口。
    let local = token(derive(&owner, root, 1, 2));
    let child = token(accord(&owner, local, dst));
    let window = Some((PageOffset::new(1), PageCount::new(2).unwrap()));
    assert_eq!(
        pend::locate(&peer.task, child).unwrap().grant().window(),
        window
    );

    let inherited = token(accord(&peer, child, owner.task.ident.id));
    assert_eq!(
        pend::locate(&owner.task, inherited)
            .unwrap()
            .grant()
            .window(),
        window,
        "a plain accord must inherit the parent's window"
    );

    // 再经一层普通 Accord 回到对端：仍不放大。
    let again = token(accord(&owner, inherited, dst));
    assert_eq!(
        pend::locate(&peer.task, again).unwrap().grant().window(),
        window
    );

    // 本地收窄一层之后，普通 Accord 继承的是**收窄后**那一段（子段只能更窄）。
    let narrow_local = token(derive(&peer, again, 1, 1));
    let narrow = token(accord(&peer, narrow_local, owner.task.ident.id));
    let tight = Some((PageOffset::new(1), PageCount::new(1).unwrap()));
    assert_eq!(
        pend::locate(&owner.task, narrow).unwrap().grant().window(),
        tight
    );
    let inherited_narrow = token(accord(&owner, narrow, dst));
    assert_eq!(
        pend::locate(&peer.task, inherited_narrow)
            .unwrap()
            .grant()
            .window(),
        tight
    );

    // 观测面：继承来的那一枚只映父那一段（长度 2 页、首 PA = 来源第 1 页），窗外取不到。
    let (whole, whole_size) = open_pair(&owner, root);
    assert!(whole > 0, "open root: {whole}");
    assert_eq!(whole_size, 4 * PAGE_SIZE);
    let source_page_1 = owner
        .team
        .space
        .nest
        .physical_at(KVirt::wrap(whole as usize + PAGE_SIZE))
        .unwrap();
    let (at, size) = open_pair(&peer, again);
    assert!(at > 0, "open inherited: {at}");
    assert_eq!(
        size,
        2 * PAGE_SIZE,
        "an inherited window opens only its window"
    );
    assert_eq!(
        peer.team.space.nest.physical_at(KVirt::wrap(at as usize)),
        Some(source_page_1),
        "the inherited window still starts at the source's first page in it"
    );
    let acquire = |token: PieToken, offset: usize, count: usize| {
        peer.call(EnvCall::Memory(MemoryCall::Acquire {
            source: AcquireSource::Source {
                token,
                offset,
                count,
            },
            retention: PieToken::NONE,
        }))
    };
    assert_eq!(acquire(again, 0, 1), MemoryFail::Denied.code() as isize);
    assert_eq!(acquire(again, 2, 2), MemoryFail::Denied.code() as isize);
    assert!(
        acquire(again, 1, 2) > 0,
        "acquire inside an inherited window"
    );
    assert_eq!(
        acquire(inherited_narrow, 2, 1),
        MemoryFail::Denied.code() as isize,
        "the narrowed branch stays narrowed after inheriting"
    );

    // 继承出来的那一枚交出的仍只是**那一段窗口**：独立子资源正好是它，不是整段来源
    // （窗口的规矩跟的是分支，不是哪一次派生）。
    let derived = create_pole(&peer.task, reference(acquire(again, 1, 2) as usize), true)
        .expect("an inherited window derives its own sub-resource");
    let root_meta = pend::locate(&owner.task, root)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    let window_range = root_meta
        .source()
        .range(PageOffset::new(1), PageCount::new(2).unwrap())
        .unwrap();
    let derived_meta = pend::locate(&peer.task, derived)
        .unwrap()
        .mail()
        .pole()
        .unwrap();
    assert!(
        derived_meta.source().same_range(&window_range),
        "the sub-resource is exactly the inherited window"
    );

    // 最终回收：撤销远端那一枚会带走继承与收窄的后代，本地派生根随后单独放下。
    assert_eq!(pie_call(&owner, PieCall::Revoke { dst, token: child }), 0);
    for (task, token) in [
        (&peer.task, child),
        (&owner.task, inherited),
        (&peer.task, again),
        (&owner.task, narrow),
        (&peer.task, inherited_narrow),
    ] {
        assert!(pend::locate(task, token).is_none());
    }
    assert!(pend::locate(&owner.task, local).is_some());
    assert_eq!(
        pie_call(&owner, PieCall::Revoke { dst: owner.task.ident.id, token: local }),
        0
    );
    assert!(pend::locate(&owner.task, local).is_none());
    assert!(pend::locate(&owner.task, root).is_some());
}

/// 本地精确派生：目标固定为调用者，窗口继承或按来源起点收窄。
///
/// 走真实调用面（`PieCall::Derive` / class 7 slot 13），并复用同一条 pend 授权发布：
/// VEST、权限子集、ONLY 移交、父子撤销都由生产实现判定，这里只量它的结果。
pub fn local_derive() {
    use env::{GrantRange, NoleCall, PieCall, PieFail, PieToken, PoleCall};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let owner = Caller::new(false);
    let peer = Caller::new(false);
    let subset = Permission::FETCH | Permission::STORE | Permission::VEST;
    let pages = |offset: usize, count: usize| GrantRange::Pages { offset, count };
    let token = |code: isize| {
        assert!(code > 0, "token: {code}");
        PieToken::mint(code as usize)
    };

    // 一份 4 页来源 → 根（整段，无窗口）。
    let (_, _, source_reference) = owner.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let root = token(pole_call(
        &owner,
        PoleCall::Unseal {
            reference: source_reference,
            shared: true,
        },
    ));
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release {
            reference: source_reference
        })),
        0
    );

    // 本地派生 (1,2)：目标是调用者，父是 root。
    let child = token(pie_call(
        &owner,
        PieCall::Derive {
            src: root,
            subset,
            mark: env::Mark::NONE,
            range: pages(1, 2),
        },
    ));
    let child_pie = pend::locate(&owner.task, child).unwrap();
    assert_eq!(
        child_pie.grant().window(),
        Some((PageOffset::new(1), PageCount::new(2).unwrap()))
    );
    assert_eq!(child_pie.sire(), Some(root));
    assert!(child_pie.lord().ptr_eq(&Arc::downgrade(&owner.task)));

    // 嵌套本地派生只收窄；越出父窗口的请求不落地。
    let nested = token(pie_call(
        &owner,
        PieCall::Derive {
            src: child,
            subset,
            mark: env::Mark::NONE,
            range: pages(1, 1),
        },
    ));
    assert_eq!(
        pend::locate(&owner.task, nested).unwrap().grant().window(),
        Some((PageOffset::new(1), PageCount::new(1).unwrap()))
    );
    let before = owner.task.pend.pies.lock().len();
    for range in [pages(0, 1), pages(3, 1), pages(1, 3)] {
        assert_eq!(
            pie_call(
                &owner,
                PieCall::Derive {
                    src: child,
                    subset,
                    mark: env::Mark::NONE,
                    range,
                },
            ),
            PieFail::Denied.code(),
            "widening local derive {range:?} must be refused"
        );
        assert_eq!(owner.task.pend.pies.lock().len(), before);
    }

    // 继承父窗口：不请求就是父那一段。
    let inherited = token(pie_call(
        &owner,
        PieCall::Derive {
            src: child,
            subset,
            mark: env::Mark::NONE,
            range: GrantRange::Inherit,
        },
    ));
    assert_eq!(
        pend::locate(&owner.task, inherited)
            .unwrap()
            .grant()
            .window(),
        Some((PageOffset::new(1), PageCount::new(2).unwrap()))
    );

    let transferred = token(pie_call(
        &owner,
        PieCall::Accord {
            src: nested,
            dst: peer.task.ident.id,
            subset,
            mark: env::Mark::NONE,
        },
    ));
    assert_eq!(
        pend::locate(&peer.task, transferred).unwrap().grant().window(),
        Some((PageOffset::new(1), PageCount::new(1).unwrap()))
    );
    let (address, size) = open_pair(&peer, transferred);
    assert!(address > 0);
    assert_eq!(size, PAGE_SIZE);

    // 空窗口在真实解码行就被拒（返回 Unknown，不进入 dispatch）。
    assert_eq!(
        owner.raw(
            (7usize << 32) | 13,
            [root.get(), subset.bits() as usize, 0, 1, 0, 0],
        ),
        env::DispatchFail::Unknown.code()
    );

    // 无 VEST 的派生枚不能再授出；子集不能加 ONLY。
    let bare = token(pie_call(
        &owner,
        PieCall::Derive {
            src: root,
            subset: subset & !Permission::VEST,
            mark: env::Mark::NONE,
            range: GrantRange::Inherit,
        },
    ));
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Derive {
                src: bare,
                subset,
                mark: env::Mark::NONE,
                range: GrantRange::Inherit,
            },
        ),
        PieFail::Denied.code()
    );
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Derive {
                src: root,
                subset: subset | Permission::ONLY,
                mark: env::Mark::NONE,
                range: GrantRange::Inherit,
            },
        ),
        PieFail::Denied.code()
    );

    // 非 Pole 资源没有授权页坐标：带 Pages 的本地派生拒绝。
    let nole = token(with_running(owner.task.clone(), || {
        owner.call(EnvCall::Nole(NoleCall::Unseal))
    }));
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Derive {
                src: nole,
                subset,
                mark: env::Mark::NONE,
                range: pages(0, 1),
            },
        ),
        PieFail::Denied.code()
    );

    // ONLY 源本地派生即移交：源不可再用，撤销子后复原。
    let (_, _, exclusive_reference) = owner.attach_alloc_reference(4 * PAGE_SIZE, 6);
    let exclusive = token(pole_call(
        &owner,
        PoleCall::Unseal {
            reference: exclusive_reference,
            shared: false,
        },
    ));
    assert_eq!(
        owner.call(EnvCall::Memory(MemoryCall::Release {
            reference: exclusive_reference
        })),
        0
    );
    let exclusive_subset = subset | Permission::ONLY;
    let exclusive_child = token(pie_call(
        &owner,
        PieCall::Derive {
            src: exclusive,
            subset: exclusive_subset,
            mark: env::Mark::NONE,
            range: GrantRange::Inherit,
        },
    ));
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Derive {
                src: exclusive,
                subset: exclusive_subset,
                mark: env::Mark::NONE,
                range: GrantRange::Inherit,
            },
        ),
        PieFail::HandedOver.code()
    );
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Revoke {
                dst: owner.task.ident.id,
                token: exclusive_child,
            },
        ),
        0
    );
    assert!(
        pie_call(
            &owner,
            PieCall::Derive {
                src: exclusive,
                subset: exclusive_subset,
                mark: env::Mark::NONE,
                range: GrantRange::Inherit,
            },
        ) > 0,
        "revoking the only child restores the exclusive source"
    );

    // 本地先精确派生，再普通 Accord 跨任务；对端本地派生继承的是那一枚的精确窗口。
    let remote_source = token(pie_call(
        &owner,
        PieCall::Derive {
            src: root,
            subset,
            mark: env::Mark::NONE,
            range: pages(1, 3),
        },
    ));
    let remote = token(pie_call(
        &owner,
        PieCall::Accord {
            src: remote_source,
            dst: peer.task.ident.id,
            subset,
            mark: env::Mark::NONE,
        },
    ));
    let remote_local = token(pie_call(
        &peer,
        PieCall::Derive {
            src: remote,
            subset,
            mark: env::Mark::NONE,
            range: GrantRange::Inherit,
        },
    ));
    assert_eq!(
        pend::locate(&peer.task, remote_local)
            .unwrap()
            .grant()
            .window(),
        Some((PageOffset::new(1), PageCount::new(3).unwrap()))
    );

    // 父 Revoke 清掉本地子孙，不伤根和另一侧兄弟。
    assert_eq!(
        pie_call(
            &owner,
            PieCall::Revoke {
                dst: owner.task.ident.id,
                token: child,
            },
        ),
        0
    );
    assert!(pend::locate(&owner.task, child).is_none());
    assert!(pend::locate(&owner.task, nested).is_none());
    assert!(pend::locate(&owner.task, inherited).is_none());
    assert!(pend::locate(&peer.task, transferred).is_none());
    assert!(pend::locate(&owner.task, root).is_some());
    assert!(pend::locate(&peer.task, remote_local).is_some());
}

/// 一次 `Open` 的两个返回值（地址与长度）：`Open` 的调用者取自运行槽，
/// 故这里自己把任务放上运行槽，不等同 `Caller::pair` 的那一手。
fn open_pair(caller: &Caller, token: env::PieToken) -> (isize, usize) {
    let EnvCall::Pole(call) = EnvCall::Pole(env::PoleCall::Open { token }) else {
        unreachable!()
    };
    let (slot, regs) = (call.slot(), call.pack());
    let frame = unsafe { &mut *(caller.task.ident.frame_pa.as_usize() as *mut TrapContext) };
    for (i, value) in regs.into_iter().enumerate() {
        frame.gpr.set_x(Gprs::A0 + i, value);
    }
    frame.gpr.set_x(Gprs::A7, slot);
    let ident = caller.task.ident.clone();
    with_running(caller.task.clone(), || {
        assert!(envcall::dispatch(frame, ident).is_some());
    });
    (frame.gpr.x(Gprs::A0) as isize, frame.gpr.x(Gprs::A1))
}

/// class 7 的调用者取自运行槽，不是派发时那个 `ident`：夹具每一手都经 `with_running`。
fn pie_call(caller: &Caller, call: env::PieCall) -> isize {
    with_running(caller.task.clone(), || caller.call(EnvCall::Pie(call)))
}

/// class 10 同理：Pole 域的每一格也从运行槽取调用者。
fn pole_call(caller: &Caller, call: env::PoleCall) -> isize {
    with_running(caller.task.clone(), || caller.call(EnvCall::Pole(call)))
}

pub fn memory_pole() {
    use crate::core::memory::flat::Frame;
    use crate::entry::call::pole::create_pole;
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
    let delegated = acquire(&service, env::AcquireSource::Reference(window), shared);
    assert!(
        delegated > 0,
        "the delegated child authorizes its holder: {delegated}"
    );
    assert_eq!(
        acquire(&service, env::AcquireSource::Reference(window), cap.token()),
        env::MemoryFail::Denied.code(),
        "the parent cap is not in this account"
    );
    assert_eq!(
        pend::revoke(&system.task, &Arc::downgrade(&service.task), shared).unwrap(),
        1
    );
    assert_eq!(
        acquire(&service, env::AcquireSource::Reference(window), shared),
        env::MemoryFail::Denied.code(),
        "a revoked child authorizes no new retention"
    );
    for (caller, handle) in [
        (&system, source),
        (&system, held),
        (&service, service_source),
        (&service, delegated),
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
    let memory = crate::entry::call::pole::create_pole(&a.task, reference, true).unwrap();
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

/// 别的任务用自己的私有来源铸一根根，并把一枚子 Pie 交给 `me`；回答 `(根, 子)`。
///
/// 交给 `me` 的那一枚 `sire` 非空：它不是 root token，拿它作不了主。
fn grant_pair(giver: &Caller, me: &Caller, pages: usize) -> (env::PieToken, env::PieToken) {
    use env::{Mark, Permission, PieToken};
    let source = acquire_fresh(giver, pages);
    assert!(source > 0, "giver source: {source}");
    materialize_page(giver, source as usize, 0);
    let root = crate::entry::call::pole::create_pole(&giver.task, reference(source as usize), true)
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
    (root, PieToken::mint(child))
}

/// 别的任务用自己的私有来源铸一根根，并把一枚子 Pie 交给 `me`。
///
/// 交给 `me` 的那一枚 `sire` 非空：它不是 root token，拿它作不了主。
fn grant_to(giver: &Caller, me: &Caller, pages: usize) -> env::PieToken {
    grant_pair(giver, me, pages).1
}

/// 内核铸的一份独立保留授权；用户入口拿不到，夹具直铸才能走到保留面。
fn retention_cap(caller: &Caller) -> env::PieToken {
    use crate::core::work::mail::nole::NoleMeta;
    let pie = pend::Pie::try_new(
        Mail::Nole(NoleMeta::mint_retention(caller.task.ident.id).expect("retention cap")),
        env::Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST,
        None,
    )
    .expect("pie");
    let token = pie.token();
    caller.task.pend.pies.lock().push(pie);
    token
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
        crate::entry::call::pole::create_pole(&user.task, reference(root_source as usize), true)
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
        crate::entry::call::pole::create_pole(&user.task, reference(narrow_source as usize), true)
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
        crate::entry::call::pole::create_pole(&user.task, reference(exclusive as usize), false)
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
        crate::entry::call::pole::create_pole(&user.task, reference(sealed_source as usize), true)
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
    let cap = retention_cap(&user);
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
    let cap = retention_cap(&user);
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

/// 撤页确认失败留在表上的那一笔，归下一个要用这张表的入口结清。
///
/// ① 结得清：真 `c.ebreak` 按两字节推进，答案写回；
/// ② 连那一次也失败：不推进 `sepc`、不写答案、不做这一次调用，显式判死；
/// ③ 表干净、入口那个字节却读不出来：同样不推进、不写答案、显式判死；
/// ④ 四字节 `ebreak` 按四字节推进。
pub fn sync_failure() {
    crate::core::work::room::scheduler::init().expect("scheduler init");
    use crate::core::memory::nest::identity::fail_next_shootdown;
    use crate::core::work::room::identity::Identity;

    // 一段可执行的真指令：`code` 是两字节 `c.ebreak`，`code + 4` 是四字节 `ebreak`。
    fn code_fixture() -> (Caller, usize) {
        let caller = Caller::new(false);
        let (_, at) = caller.attach_alloc(PAGE_SIZE, 6);
        let code = at + PAGE_SIZE / 4;
        assert!(
            caller.team.space.copy_out(
                &[0x02, 0x90, 0x00, 0x00, 0x73, 0x00, 0x10, 0x00],
                KVirt::wrap(code)
            ),
            "c.ebreak / ebreak at {code:#x}"
        );
        assert_eq!(caller.set_access_at(at, PAGE_SIZE, 10), 0, "R|X");
        (caller, code)
    }

    // 再在表上留一笔待结算的失效：一次撤页确认失败（真安装真失败）。
    fn arm_failed_sync(caller: &Caller) {
        let scratch = caller.scratch_page();
        let handle = caller.call(EnvCall::Memory(MemoryCall::Acquire {
            source: env::AcquireSource::Allocation(
                env::Allocation::new(PAGE_SIZE, PAGE_SIZE).expect("allocation"),
            ),
            retention: env::PieToken::NONE,
        }));
        assert!(handle > 0, "acquire: {handle}");
        caller.write_words(scratch, &[handle as usize, 0, 1]);
        caller.write_words(scratch + 32, &[0, 0, PAGE_SIZE]);
        fail_next_shootdown(caller.team.space.nest.asid());
        let attach = MemoryCall::Attach {
            range: VirtAddr::new(scratch),
            team: env::TeamId::new(0),
            placement: VirtAddr::new(scratch + 32),
            access: 6,
            control: env::ViewControl::Caller,
        };
        assert_eq!(
            caller.raw(attach.slot(), attach.pack()),
            env::MemoryFail::Busy.code(),
            "armed sync fails the install"
        );
        assert!(
            caller.team.space.nest.has_pending_invalidation(),
            "a failed sync keeps its responsibility"
        );
    }

    fn frame_of(caller: &Caller) -> &mut TrapContext {
        unsafe { &mut *(caller.task.ident.frame_pa.as_usize() as *mut TrapContext) }
    }

    // 判死必须是显式的 `Fault`：`None` 交回后入口会换出它，不许被当成 clean。
    fn killed(caller: &Caller) {
        assert!(caller.task.dead(), "the task is doomed");
        assert_eq!(
            caller.task.state.lock().dying(),
            Some((env::ExitCause::Fault, env::EXIT_FAULT)),
            "explicit Fault, never a clean reap"
        );
    }

    // ① 结得清：先结清再执行。
    {
        let (caller, code) = code_fixture();
        arm_failed_sync(&caller);
        let id = caller.task.ident.id;
        let frame = frame_of(&caller);
        frame.sepc = KVirt::wrap(code);
        frame.gpr.set_x(Gprs::A7, UnitCall::SelfId.slot());
        assert!(
            crate::entry::trap::envcall(frame, Some(Identity::Live(caller.task.ident.clone())))
                .is_some(),
            "settled: the call is served"
        );
        assert!(
            !caller.team.space.nest.has_pending_invalidation(),
            "the entry settled the pending sync first"
        );
        assert_eq!(
            frame.sepc.as_usize(),
            code + 2,
            "c.ebreak advances two bytes"
        );
        assert_eq!(frame.gpr.x(Gprs::A0), id.get(), "answer written back");
    }

    // ② 结清那一次也失败：没有这一次调用，也没有错位的 `sepc`。
    {
        let (caller, code) = code_fixture();
        arm_failed_sync(&caller);
        fail_next_shootdown(caller.team.space.nest.asid());
        let frame = frame_of(&caller);
        frame.sepc = KVirt::wrap(code);
        frame.gpr.set_x(Gprs::A0, 0x5eed);
        frame.gpr.set_x(Gprs::A7, UnitCall::SelfId.slot());
        assert!(
            crate::entry::trap::envcall(frame, Some(Identity::Live(caller.task.ident.clone())))
                .is_none(),
            "an unconfirmed sync cannot be served"
        );
        assert_eq!(frame.sepc.as_usize(), code, "sepc untouched");
        assert_eq!(frame.gpr.x(Gprs::A0), 0x5eed, "no answer written");
        assert!(
            caller.team.space.nest.has_pending_invalidation(),
            "the responsibility stays"
        );
        killed(&caller);
    }

    // ③ 表干净、字节却读不出来（那一段没有可执行权限）：同样不许推进。
    {
        let (caller, _) = code_fixture();
        assert!(
            !caller.team.space.nest.has_pending_invalidation(),
            "clean table"
        );
        let data = caller.scratch_page();
        let frame = frame_of(&caller);
        frame.sepc = KVirt::wrap(data);
        frame.gpr.set_x(Gprs::A0, 0x5eed);
        frame.gpr.set_x(Gprs::A7, UnitCall::SelfId.slot());
        assert!(
            crate::entry::trap::envcall(frame, Some(Identity::Live(caller.task.ident.clone())))
                .is_none(),
            "an unreadable entry cannot be served"
        );
        assert_eq!(frame.sepc.as_usize(), data, "sepc untouched");
        assert_eq!(frame.gpr.x(Gprs::A0), 0x5eed, "no answer written");
        killed(&caller);
    }

    // ④ 四字节 `ebreak` 按四字节推进。
    {
        let (caller, code) = code_fixture();
        let id = caller.task.ident.id;
        let frame = frame_of(&caller);
        frame.sepc = KVirt::wrap(code + 4);
        frame.gpr.set_x(Gprs::A7, UnitCall::SelfId.slot());
        assert!(
            crate::entry::trap::envcall(frame, Some(Identity::Live(caller.task.ident.clone())))
                .is_some(),
            "an ebreak is served"
        );
        assert_eq!(
            frame.sepc.as_usize(),
            code + 8,
            "ebreak advances four bytes"
        );
        assert_eq!(frame.gpr.x(Gprs::A0), id.get(), "answer written back");
    }
}

// ---- 封印与撤销（Pole 的 `Seal` / class 7 的 `Revoke`）：真实解码派发，调用者来自运行槽 ----
//
// `Seal` / `Revoke` 的调用者取自 `current().running_task()`，不是派发时那个
// `ident`。故每一下都经 `with_running` 把被测任务真正放上本核运行槽，再走
// `EnvCall` 解码派发（`Caller.raw` 是服务派发，刻意绕过 `trap::envcall`）。

/// 真实 `PoleCall::Seal` 的管理面：成功、主人不符，与两种明确的 `Busy`。
pub fn pole_seal_authority() {
    use env::{PieToken, PoleCall, PoleFail};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let me = Caller::new(false);
    let lord = Caller::new(false);
    let seal = |token| {
        with_running(me.task.clone(), || {
            me.call(EnvCall::Pole(PoleCall::Seal { token }))
        })
    };

    // 本任务自己的私有整段来源铸出来的根：这段 Pole 的主人就是本任务；创建时那段
    // 关系（Input）就是它名下的图。
    let (_, at, source) = me.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let root = crate::entry::call::pole::create_pole(&me.task, source, true).expect("root");
    assert_eq!(
        me.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let meta = pend::locate(&me.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    assert!(me.team.space.nest.translate(KVirt::wrap(at)).is_some());

    // 主人不是调用者：别的任务铸的根交给我那一枚，我作不了主。
    let child = grant_to(&lord, &me, 2);
    let lord_meta = pend::locate(&me.task, child)
        .expect("child pie")
        .mail()
        .pole()
        .expect("a pole");
    assert_eq!(seal(child), PoleFail::Denied.code());
    assert!(
        lord_meta.alive(),
        "a refused seal must not deactivate the Pole"
    );
    assert!(pend::locate(&me.task, child).expect("child pie").alive());

    // 手里没这一枚：当场拒。
    assert_eq!(seal(PieToken::mint(0x5eed)), PoleFail::Denied.code());

    // 资源本身还活、这一枚的 Grant 已经不答话（授权失效那一档，不是补关）：原属主
    // 也不行，且不许把资源失活。Grant 用生产 API 作废，token 仍留在表里。
    let (_, at2, source2) = me.attach_alloc_reference(PAGE_SIZE, 6);
    let root2 = crate::entry::call::pole::create_pole(&me.task, source2, true).expect("root");
    assert_eq!(
        me.call(EnvCall::Memory(MemoryCall::Release { reference: source2 })),
        0
    );
    let meta2 = pend::locate(&me.task, root2)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    pend::locate(&me.task, root2)
        .expect("root pie")
        .grant()
        .invalidate();
    assert!(meta2.alive(), "the resource itself is still alive");
    assert!(
        !pend::locate(&me.task, root2).expect("root pie").alive(),
        "its Grant answers nothing"
    );
    assert_eq!(seal(root2), PoleFail::Denied.code());
    assert!(
        meta2.alive(),
        "a refused seal must not deactivate the resource"
    );
    assert!(
        me.team.space.nest.translate(KVirt::wrap(at2)).is_some(),
        "a refused seal must not tear a map down"
    );
    pend::release(&me.task, root2).unwrap();

    // 这一段已经交给别的 team（staged 保留）：明确 Busy，不失活、不撤图。
    let view = me
        .team
        .space
        .nest
        .probe(KVirt::wrap(at))
        .expect("the input view");
    assert!(
        meta.record(
            root,
            &me.team.space,
            view.clone(),
            Some(lord.team.id),
            false
        )
        .is_ok(),
        "stage the relation under another team"
    );
    assert_ne!(meta.reserved(), 0);
    assert_eq!(seal(root), PoleFail::Busy.code());
    assert!(meta.alive(), "a Busy seal must not deactivate the Pole");
    assert!(
        me.team.space.nest.translate(KVirt::wrap(at)).is_some(),
        "a Busy seal must not tear a map down"
    );
    meta.detach_staging(root, lord.team.id);
    assert_eq!(meta.reserved(), 0);

    // Pole 自己的操作租约被占：同样明确 Busy。
    let operation = pole::PoleOperation::acquire(&meta).expect("the Pole operation lease");
    assert_eq!(seal(root), PoleFail::Busy.code());
    assert!(meta.alive());
    assert!(me.team.space.nest.translate(KVirt::wrap(at)).is_some());
    drop(operation);

    // 成功：0；Pole 失活、名下关系关掉，而它那枚 Grant 仍答话。
    assert_eq!(seal(root), 0);
    assert!(!meta.alive(), "the seal lands on the Pole itself");
    assert!(!pend::locate(&me.task, root).expect("root pie").alive());
    assert!(me.team.space.nest.translate(KVirt::wrap(at)).is_none());
    assert!(
        meta.grant(root)
            .is_some_and(|grant| grant.permission().is_some()),
        "the root Grant still answers after the seal"
    );
    me.team.space.nest.audit();
    drop(view);
    pend::release(&me.task, root).unwrap();
}

/// 真实 `PoleCall::Seal` 之后：这段 Pole 当场不再是管理者。
///
/// 口径是**管理拒绝先于物理资格**，不是“Seal 把一次成功变成拒绝”：两端都用带独立
/// 保留的窗口，那份保留自己就压着源页，故 Seal **之前**同一次迁移本来就答 `Busy`
/// （源槽不是空闲的承载）；Seal **之后**同一次调用的答案换成 `Denied`——入口的
/// `endpoint` 先问管理判据（`manages`），而驻留那一格要到 `Transaction::claim` 才问。
/// 真 0 成功面由姊妹用例 `pole_seal_keeps_transferred_page` 承载，本例不声称 0→Denied。
///
/// 保留窗口在 Seal 之后仍解析得动（保留自己强持来源，不看 Pole），故副作用基线取
/// Seal 完成后，逐项比对被拒前后的整页读数与帧账。
pub fn pole_seal_refuses_transfer() {
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use env::{AcquireSource, MemoryFail, PoleCall, ReferenceRange};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let me = Caller::new(false);
    let (_, _, source) = me.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let root = crate::entry::call::pole::create_pole(&me.task, source, true).expect("root");
    assert_eq!(
        me.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let meta = pend::locate(&me.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    let seal_root = || {
        with_running(me.task.clone(), || {
            me.call(EnvCall::Pole(PoleCall::Seal { token: root }))
        })
    };
    let held = me.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: root,
            offset: 0,
            count: 2,
        },
        retention: env::PieToken::NONE,
    }));
    assert!(held > 0, "the root lends its window: {held}");
    let cap = retention_cap(&me);
    let window = me.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Reference(
            ReferenceRange::new(reference(held as usize), 0, 2).expect("window"),
        ),
        retention: cap,
    }));
    assert!(window > 0, "a retained window: {window}");
    let pa = slot_at(&window_of(&me, window as usize), 0);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));
    let seats = slots_of(&window_of(&me, window as usize));

    // Seal 之前的**同配置对照**：这份保留正压着源页 ⇒ 源槽不是空闲承载 ⇒ `Busy`。
    // 这是占槽前的早错：一格槽没被改、一格帧没动、也没有关系被这次移交动过。
    let frames = frame_occupied() - block_occupied();
    assert_eq!(
        me.call(transfer_call(window as usize, 0, window as usize, 1)),
        MemoryFail::Busy.code(),
        "a residency over the source page is not an idle carrier"
    );
    assert_eq!(slots_of(&window_of(&me, window as usize)), seats);
    assert_eq!(frame_occupied() - block_occupied(), frames);
    assert!(!me.team.space.nest.has_pending_invalidation());

    // 真实 Seal：成功；Pole 失活，而它的 Grant 仍答话。
    assert_eq!(seal_root(), 0);
    assert!(!meta.alive(), "the seal lands on the Pole itself");
    assert!(
        meta.grant(root)
            .is_some_and(|grant| grant.permission().is_some()),
        "the root Grant still answers even though the Meta is dead"
    );

    // 基线取 Seal 之后：保留窗口仍解析得动，承载读数与帧账都从这里量。
    // Seal 本来就撤它名下映射、会把那些表页还回分配器 ⇒ **不**拿跨 Seal 的帧账当
    // 不变断言；承载本身有没有动，看槽位读数（下面那句）。
    let baseline = slots_of(&window_of(&me, window as usize));
    assert_eq!(baseline, seats, "the Seal moved no carrier");
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));
    let after = frame_occupied() - block_occupied();
    assert!(!me.team.space.nest.has_pending_invalidation());

    assert_eq!(
        me.call(transfer_call(window as usize, 0, window as usize, 1)),
        MemoryFail::Denied.code(),
        "after the Seal the management gate refuses before the physical one is consulted"
    );
    // 被拒的迁移不留任何副作用：读数与基线逐项相同。
    assert_eq!(slots_of(&window_of(&me, window as usize)), baseline);
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));
    assert_eq!(frame_occupied() - block_occupied(), after);
    assert!(!me.team.space.nest.has_pending_invalidation());
    me.team.space.nest.audit();
}

/// 真实 `PoleCall::Seal` 只关**自己名下**的关系：已经交进目标槽的整页照旧在。
///
/// 从这条 Pole 的窗口里把一页交进本任务一份**私有**空窗口：承载与整页内容一起落到
/// 目标槽，且移动不新分配任何帧。Seal 之后，Pole 自己那条关系关掉，别的普通映射
/// 照旧回话，而交出去的那一页留在目标槽里。
///
/// 本例也是这条入口的**真 0 成功面**（两端合法、没有驻留压页）——`pole_seal_refuses_transfer`
/// 那条“Seal 前 `Busy` → Seal 后 `Denied`”的对照不拿本例当同配置。
pub fn pole_seal_keeps_transferred_page() {
    use super::transfer::{fill_pattern, pattern_holds};
    use crate::core::memory::flat::allocator::statistics::{block_occupied, frame_occupied};
    use env::{AcquireSource, PoleCall};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let me = Caller::new(false);
    // 一段与这条 Pole 无关的普通映射：Seal 不许动它。
    let (_, spare) = me.attach_alloc(PAGE_SIZE, 6);
    let (_, at, source) = me.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let root = crate::entry::call::pole::create_pole(&me.task, source, true).expect("root");
    assert_eq!(
        me.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let meta = pend::locate(&me.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    let held = me.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: root,
            offset: 0,
            count: 2,
        },
        retention: env::PieToken::NONE,
    }));
    assert!(held > 0, "the root lends its window: {held}");
    let source_range = window_of(&me, held as usize);
    let target = acquire_fresh(&me, 2);
    assert!(target > 0, "a fresh private window: {target}");
    let target_range = window_of(&me, target as usize);
    let pa = slot_at(&source_range, 0);
    fill_pattern(PhysAddr::from_raw(pa));

    // 移交成功：承载与整页内容一起落进目标空槽，不新分配任何帧。
    let frames = frame_occupied() - block_occupied();
    assert_eq!(
        me.call(transfer_call(held as usize, 0, target as usize, 0)),
        0,
        "a resident page moves into a fresh slot"
    );
    assert_eq!(
        slots_of(&target_range),
        alloc::vec![(0, pa)],
        "the target seat holds the same physical start"
    );
    assert!(
        pattern_holds(PhysAddr::from_raw(pa)),
        "all 4096 bytes came along"
    );
    assert!(
        tombstone(&source_range, 0),
        "the source page becomes a tombstone"
    );
    assert_eq!(
        frame_occupied() - block_occupied(),
        frames,
        "the carrier moved; no frame was allocated or lost"
    );

    // 真实 Seal：它自己那条关系关掉，别的照旧回话；交出去的整页留在目标槽。
    let sealed = with_running(me.task.clone(), || {
        me.call(EnvCall::Pole(PoleCall::Seal { token: root }))
    });
    assert_eq!(sealed, 0);
    assert!(!meta.alive());
    assert!(
        me.team.space.nest.translate(KVirt::wrap(at)).is_none(),
        "the Pole's own relation is closed"
    );
    assert!(
        me.team.space.nest.translate(KVirt::wrap(spare)).is_some(),
        "an unrelated relation of the same task survives"
    );
    assert_eq!(
        slots_of(&target_range),
        alloc::vec![(0, pa)],
        "the target seat keeps the same physical start"
    );
    assert!(
        pattern_holds(PhysAddr::from_raw(pa)),
        "the transferred bytes are intact"
    );
    me.team.space.nest.audit();
}

/// 真实 `PoleCall::Seal` 部分失败与**补关**：首次失活落地但关图同步失败时，原属主可以
/// 对同一枚仍在表里的 token 重发 Seal 把关闭补完。
///
/// ① 注入一次撤页确认失败（`close_maps` 里的同步），Seal 当场拒 `Busy`：失活已落地、
///    不回滚，关闭责任留在 Pole 上，那段占用禁提前复用；
/// ② 换**别的任务**拿同一枚 Pole 的另一枚 token 补关 → 照旧 `Denied`，且不动原处；
/// ③ 原属主重发 Seal：这次走真入口 `trap::envcall`（真 `c.ebreak` 长度与推进），
///    它先结清那张表上待结的失效，再派发这一次调用 → `0`，图关完；
/// ④ 关完之后再发一次：既定精确返回是 `Dead`（资源已失活且无图可关）；
/// ⑤ 失活没有被撤销，也没有恢复任何活权限（来源的管理判据当场从真变假）；Grant 是
///    这一枚 Pie 自己的，Seal 不动它，token 走到尽头（`release`）它才随之消失。
pub fn pole_seal_partial_failure() {
    use crate::core::memory::nest::identity::fail_next_shootdown;
    use crate::core::work::room::identity::Identity;
    use env::{AcquireSource, Mark, Permission, PieToken, PoleCall, PoleFail};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let me = Caller::new(false);
    let stranger = Caller::new(false);
    let (_, at, source) = me.attach_alloc_reference(2 * PAGE_SIZE, 6);
    let root = crate::entry::call::pole::create_pole(&me.task, source, true).expect("root");
    assert_eq!(
        me.call(EnvCall::Memory(MemoryCall::Release { reference: source })),
        0
    );
    let meta = pend::locate(&me.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    assert!(me.team.space.nest.translate(KVirt::wrap(at)).is_some());
    let seal = || {
        with_running(me.task.clone(), || {
            me.call(EnvCall::Pole(PoleCall::Seal { token: root }))
        })
    };
    // 补关只认原属主：同一枚 Pole 的另一枚 token 落在别的任务手里（`accord` 要给
    // 活资源，故先发）。
    let outsider = PieToken::mint(
        pend::accord(
            &me.task,
            root,
            &Arc::downgrade(&stranger.task),
            Permission::FETCH | Permission::STORE | Permission::VEST,
            Mark::NONE,
        )
        .expect("a child grant"),
    );
    // 这一段来源的管理判据（迁移两端用的就是它）：先取一份引用，好看到它随失活翻面。
    let held = me.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: root,
            offset: 0,
            count: 2,
        },
        retention: env::PieToken::NONE,
    }));
    assert!(held > 0, "the root lends its window: {held}");
    let manages = || {
        let hold = me.task.pend.references.get(held as usize).expect("hold");
        let _serial = me.task.pend.lock();
        hold.source().manages(&me.task)
    };
    assert!(manages(), "a live Pole manages its own source");

    // 重试那一格要的真指令先备齐（这一步自己会结算失效面，故必须在注入之前）。
    let (_, code_at) = me.attach_alloc(PAGE_SIZE, 6);
    let code = code_at + PAGE_SIZE / 4;
    assert!(
        me.team.space.copy_out(
            &[0x02, 0x90, 0x00, 0x00, 0x73, 0x00, 0x10, 0x00],
            KVirt::wrap(code)
        ),
        "c.ebreak / ebreak at {code:#x}"
    );
    assert_eq!(me.set_access_at(code_at, PAGE_SIZE, 10), 0, "R|X");
    assert!(!me.team.space.nest.has_pending_invalidation());

    // ① 下一次撤页确认失败：Seal 的 `close_maps` 当场拒。
    fail_next_shootdown(me.team.space.nest.asid());
    assert_eq!(
        seal(),
        PoleFail::Busy.code(),
        "an unconfirmed close is retryable"
    );
    // 失活已经落地，不回滚。
    assert!(!meta.alive(), "the seal landed and is not rolled back");
    assert!(!pend::locate(&me.task, root).expect("root pie").alive());
    // 关闭/退休责任保留：名字还挂在 Pole 名下，占用禁复用，失效待结算；失活也没把
    // 管理权还给这一枚（来源的管理判据当场变假）。
    assert!(
        meta.has_maps(),
        "the close responsibility stays on the Pole"
    );
    assert!(me.team.space.nest.has_pending_invalidation());
    assert_eq!(
        super::support::reserve(&me.team.space, KVirt::wrap(at), PAGE_SIZE).err(),
        Some(NestError::NoRegion),
        "the retired span may not be reused early"
    );
    assert!(!manages(), "the landing seal restored no management");

    // ② 非属主捧着同一枚 Pole 的另一枚 token：照旧拒，不动原处。
    let stranger_seal = || {
        with_running(stranger.task.clone(), || {
            stranger.call(EnvCall::Pole(PoleCall::Seal { token: outsider }))
        })
    };
    assert_eq!(
        stranger_seal(),
        PoleFail::Denied.code(),
        "a non-owner cannot finish someone else's close"
    );
    assert!(meta.has_maps() && me.team.space.nest.has_pending_invalidation());

    // ③ 真入口重试（原属主）：先结清待结的失效、按真长度推过 `c.ebreak`，再派发。
    let frame_pa = me.task.ident.frame_pa.as_usize();
    {
        let slot = PoleCall::Seal { token: root }.slot();
        let frame = unsafe { &mut *(frame_pa as *mut TrapContext) };
        frame.sepc = KVirt::wrap(code);
        frame.gpr.set_x(Gprs::A7, slot);
        frame.gpr.set_x(Gprs::A0, root.get());
    }
    let served = with_running(me.task.clone(), || {
        let frame = unsafe { &mut *(frame_pa as *mut TrapContext) };
        crate::entry::trap::envcall(frame, Some(Identity::Live(me.task.ident.clone()))).is_some()
    });
    assert!(served, "the entry serves the call");
    assert!(
        !me.team.space.nest.has_pending_invalidation(),
        "the entry settled the pending sync first"
    );
    let frame = unsafe { &*(frame_pa as *const TrapContext) };
    assert_eq!(
        frame.sepc.as_usize(),
        code + 2,
        "c.ebreak advances two bytes"
    );
    assert_eq!(
        frame.gpr.x(Gprs::A0) as isize,
        0,
        "the owner's retry completes the close"
    );
    assert!(!meta.has_maps(), "the close responsibility is settled");
    assert!(me.team.space.nest.translate(KVirt::wrap(at)).is_none());

    // ④ 关完之后再发一次：既定精确返回是 `Dead`。
    assert_eq!(
        seal(),
        PoleFail::Dead.code(),
        "a finished seal answers Dead"
    );

    // ⑤ 失活没被撤销、管理权没回来；Grant 跟看这一枚 Pie 自己的命，`Seal` 不动它。
    assert!(!meta.alive() && !pend::locate(&me.task, root).expect("root pie").alive());
    assert!(!manages(), "the finished close restored no management");
    assert!(
        meta.grant(root)
            .is_some_and(|grant| grant.permission().is_some()),
        "the Grant belongs to the Pie; the Seal does not invalidate it"
    );
    pend::release(&me.task, root).unwrap();
    assert!(
        meta.grant(root).is_none(),
        "the Grant goes where the Pie that carried it goes"
    );
    me.team.space.nest.audit();
    stranger.team.space.nest.audit();
}

/// 真实 `PieCall::Revoke`：`dst` / `token` 与 lord 侧授权把关；撤销之后这一段
/// 连它名下关系一起消失。
///
/// 把关是**两格**：`cull` 先在目标表里找得到这一枚，再核调用者（他得是它的 lord，
/// 且自己表里还得有它的 parent token）。生产可构造的状态里，lord 关系与 parent token
/// 是同一次 `accord` 一起落进调用者表的（`Keep` 会把 sire 与 lord 一并转给上家，
/// `pend::forget` 不落单格），故**只废掉其中一格**的状态不可达：以下几条负例一律
/// 只断言联合判据把人挡住，不声称覆盖了哪一格，也不为追覆盖去伪造 token 表。
pub fn pole_revoke_removes_child() {
    use env::{AcquireSource, Mark, MemoryFail, Permission, PieCall, PieFail, PieToken};

    crate::core::work::room::scheduler::init().expect("scheduler init");
    let lord = Caller::new(false);
    let peer = Caller::new(false);
    let stranger = Caller::new(false);
    let rw = Access::READ | Access::WRITE;

    // lord 用自己的私有来源铸一根根，把两枚子 Pie 分别交给 peer 与 stranger。
    let (root, child) = grant_pair(&lord, &peer, 1);
    let other_child = PieToken::mint(
        pend::accord(
            &lord.task,
            root,
            &Arc::downgrade(&stranger.task),
            Permission::FETCH | Permission::STORE | Permission::VEST,
            Mark::NONE,
        )
        .expect("a second child grant"),
    );
    let meta = pend::locate(&lord.task, root)
        .expect("root pie")
        .mail()
        .pole()
        .expect("a pole");
    let (lord_at, _) = pole::open(&meta, root, &lord.team.space, rw).expect("lord's relation");
    let (peer_at, _) = pole::open(&meta, child, &peer.team.space, rw).expect("peer's relation");
    let lord_nest = lord.team.space.nest.clone();
    let peer_nest = peer.team.space.nest.clone();
    // peer 手里这份引用取自 child 那条分支；它自己强持不了来源（未带保留）。
    let peer_held = peer.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: child,
            offset: 0,
            count: 1,
        },
        retention: PieToken::NONE,
    }));
    assert!(peer_held > 0, "the child lends its window: {peer_held}");
    let peer_range = window_of(&peer, peer_held as usize);
    let pa = slot_at(&peer_range, 0);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));

    let revoke = |dst, token| {
        with_running(lord.task.clone(), || {
            lord.call(EnvCall::Pie(PieCall::Revoke { dst, token }))
        })
    };
    let from_peer = |dst: env::TaskId, token| {
        with_running(peer.task.clone(), || {
            peer.call(EnvCall::Pie(PieCall::Revoke { dst, token }))
        })
    };

    // dst 指错人：拿着这枚 token 的既不是他，也没有这一枚可撤（找得到这一枚是先决）。
    assert_eq!(
        revoke(stranger.task.ident.id, child),
        PieFail::Denied.code()
    );
    // 根那一枚自己不能当被撤的那一枚：`sire` 为空 ⇒ 没有 lord 侧授权。
    assert_eq!(revoke(lord.task.ident.id, root), PieFail::Denied.code());
    // lord 侧授权：该撤的那一枚归它自己的 vestor，别的任务撤不动。
    assert_eq!(
        from_peer(stranger.task.ident.id, other_child),
        PieFail::Denied.code()
    );
    // 上面几次拒绝都没有动到任何东西。
    assert!(pend::locate(&peer.task, child).is_some());
    assert!(peer_nest.translate(KVirt::wrap(peer_at)).is_some());
    assert!(pend::locate(&stranger.task, other_child).is_some());

    // 真撤销：0；子那枚与它名下关系一起收掉，别的分支不动。
    assert_eq!(revoke(peer.task.ident.id, child), 0);
    assert!(pend::locate(&peer.task, child).is_none());
    assert!(peer_nest.translate(KVirt::wrap(peer_at)).is_none());
    assert!(lord_nest.translate(KVirt::wrap(lord_at)).is_some());
    assert!(pend::locate(&stranger.task, other_child).is_some());
    assert_eq!(
        slots_of(&peer_range),
        alloc::vec![(0, pa)],
        "the revoked branch moved no carrier"
    );
    assert!(
        super::transfer::pattern_holds(PhysAddr::from_raw(pa)),
        "the source page itself is untouched"
    );

    // 撤销之后这一段连它名下关系都不在了。这次迁移被拒是**两道门一起**的结果：
    // 入口先问管理判据（那枚 root token 已不在本表），而这条分支的来源也解析不到——
    // 故不拿它单独作证是哪一道门拒的（单看管理判据的是 `pole_seal_refuses_transfer`，
    // 那里保留窗口仍解析得动）。它只说明：撤销后这一段不再是可用的一端。
    assert_eq!(
        peer.call(transfer_call(peer_held as usize, 0, peer_held as usize, 1)),
        MemoryFail::Denied.code(),
        "a revoked branch is no longer a usable end"
    );
    assert_eq!(slots_of(&peer_range), alloc::vec![(0, pa)]);
    lord.team.space.nest.audit();
    peer.team.space.nest.audit();
    stranger.team.space.nest.audit();
}

/// 跨 Task／跨 Space 的受控交错：一段来源的**另一个地址空间**里压着的关系。
///
/// 真实可达形状只有这一种：这段来源的管理者是这把根 Grant 的持有者，别的任务拿到的
/// 是 `accord` 交出的子分支——它**当不了 manager**（`manages` 要的是本表里 `sire` 为空的
/// 那一枚），但它能在**自己那个 Team 的 Space** 里 `open` 出压着同一批帧的关系。
/// 于是竞争者真出现在这几处：`Shut`（自持子分支的关闭）、`Revoke`（lord 侧收回那一枚）、
/// `doom`（退出时的 cull 与清理）、以及撤页确认失败（两个 Nest 一成一败）。
///
/// 本用例是**受控交错**：每一手都走真入口、真任务表与真 pend，但它们是先后发生的，
/// **不是并发**。真多 Hart 的同刻交错不在本夹具的证明范围（见 docs 该节）。
/// 双 manager 同来源、别的 Task 替调用者持 pend 这两类组合不可达：前者的管理器身份是
/// 私有属主或根 token 单点，后者以调用者自己的 pend 为准；不为凑矩阵造这样的状态。
pub fn cross_task_interleavings() {
    use crate::core::memory::flat::allocator::statistics;
    use crate::core::memory::flat::allocator::statistics::Kind;
    use alloc::vec::Vec;

    crate::core::work::room::scheduler::init().expect("scheduler init");
    // 热身：首次建拆一个 Team 会触发一次性的惰性分配；先走一遍再取基线。
    retire(&Caller::new(false));
    // 用例要用的那几台真 Task/Team **在基线之前**建齐：夹具给每台 Caller 的账
    // 不混进被测窗口，窗口里只量本景自己的动作（来源、Pole、关系、页槽与承载）。
    let a_lord = Caller::new(false);
    let a_peer = Caller::new(false);
    let b_lord = Caller::new(false);
    let b_peer = Caller::new(false);
    let c_lord = Caller::new(false);
    let c_peer = Caller::new(false);

    let cases: [(&str, fn(&Caller, &Caller), (&Caller, &Caller)); 4] = [
        (
            "foreign space page",
            a_manager_takes_the_foreign_page_out,
            (&a_lord, &a_peer),
        ),
        (
            "holder leaves first",
            a_holder_leaving_gives_up_the_page,
            (&b_lord, &b_peer),
        ),
        (
            "revoked branch",
            a_revoked_branch_stops_being_an_end,
            (&a_lord, &a_peer),
        ),
        (
            "failed sync",
            a_failed_sync_stays_in_its_own_nest,
            (&c_lord, &c_peer),
        ),
    ];
    // 热身：每一对 Caller 先用**一个不终结任何任务**的形状走一遍（`revoked branch`：
    // 它建拆同一批结构——来源、根、子分支、两个空间里的关系——但不叫走持窗者）。
    // 那几笔一次性的惰性分配（整页被块池建下、小块首次取用）只发生在这一对 Caller
    // 的**第一轮**里；把基线取在它之后，逐类账里剩下的才是本景自己的动作。
    //
    // **不能拿被热身那一形状本身当热身**：`holder leaves first` 的目的就是真叫走
    // 持窗者，同一对 Caller 跑不了第二轮（第二次 `accord` 会答 `Dead`）。
    for (lord, peer) in [(&a_lord, &a_peer), (&b_lord, &b_peer), (&c_lord, &c_peer)] {
        a_revoked_branch_stops_being_an_end(lord, peer);
    }

    let frames_before = statistics::frame_occupied() - statistics::block_occupied();
    let kinds_before = statistics::kinds();
    let mut off = Vec::new();
    for (name, run, (lord, peer)) in cases {
        let base = statistics::frame_occupied() - statistics::block_occupied();
        run(lord, peer);
        let now = statistics::frame_occupied() - statistics::block_occupied();
        if now != base {
            off.push((name, base, now));
        }
    }
    crate::expect!(off.is_empty(), "cross-task: per-case net: {:?}", off);

    let frames_after = statistics::frame_occupied() - statistics::block_occupied();
    crate::expect!(
        frames_after == frames_before,
        "cross-task: frame occupancy {frames_before} → {frames_after} (net must be 0)"
    );
    // 逐类账：帧、页表（`Table`）、任务/团队/空间、栈、各类帧一律 == 基线。
    //
    // `Prime`（整页被块池建下，它的账同时进 `block_occupied`，故上面那句帧净额看不见它）
    // 与 `Plain`（小块仍处于已取状态）这两格**不是线性增长**：实测同一对 Caller 同一形状
    // 连跑 6 轮，第 0 轮 ＋2/＋7，第 1..5 轮全为 0。故基线取在每对 Caller 热身一轮之后，
    // 这两格只钉**不增长**（`<=` 基线，没有正增长、也没有任意阈值），其余一律相等；
    // 本夹具自己跑出来的是两格都为 0（差值为负同样不算违规，它只说明池子收回得更多）。
    let kinds_after = statistics::kinds();
    for (kind, count) in kinds_after.nonzero() {
        let before = kinds_before.get(kind);
        if matches!(kind, Kind::Prime | Kind::Plain) {
            crate::expect!(
                count <= before,
                "cross-task: kind {}: {} → {count} (must not grow)",
                kind.name(),
                before
            );
            continue;
        }
        crate::expect!(
            count == before,
            "cross-task: kind {}: {} → {count} (net must be 0)",
            kind.name(),
            before
        );
    }

    // 夹具这边在**被测窗口之外**收尾：按真实退出终结每台，再让它们随作用域落下。
    for caller in [&a_peer, &b_peer, &c_peer, &a_lord, &b_lord, &c_lord] {
        retire(caller);
    }
}

/// 交还一份引用（只摘本表那一格，不 Detach、不等别人）。
fn release(caller: &Caller, handle: usize) {
    assert_eq!(
        caller.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(handle)
        })),
        0
    );
}

/// 每个用例末尾的同一个结清点：先交还本用例取的引用，再索引/发布记录/PTE 自检，
/// 最后让两个 Nest 把待结的失效结清。
///
/// 结清是账的一部分：撤页与关图把退役的承载交给那一个 Nest，只有它真的确认过
/// （或者它的空间一起关掉）那几帧才会回来；引用交还则是调用方自己的责任。
fn settle(lord: &Caller, peer: &Caller) {
    lord.team.space.nest.audit();
    peer.team.space.nest.audit();
    lord.team
        .space
        .nest
        .retry_invalidation()
        .expect("the manager's nest settles");
    peer.team
        .space
        .nest
        .retry_invalidation()
        .expect("the holder's nest settles");
}

/// 一对：`lord` 是这段来源的管理者，`peer` 是另一个 Team（另一个 Space）里持子分支的任务。
struct AcrossSpaces {
    root: env::PieToken,
    child: env::PieToken,
    meta: Arc<pole::PoleMeta>,
    lord_handle: usize,
    peer_handle: usize,
    lord_at: usize,
    peer_at: usize,
}

impl AcrossSpaces {
    /// 两端的窗口（manager 从**根**分支取，持窗者从子分支取）。
    fn lord_window(&self, lord: &Caller) -> FlatRange {
        window_of(lord, self.lord_handle)
    }

    /// manager 自己空间里的那一条关系。
    fn lord_view(&self, lord: &Caller) -> Arc<View> {
        lord.team
            .space
            .nest
            .probe(KVirt::wrap(self.lord_at))
            .expect("the manager's relation")
    }

    /// 持窗者自己空间里的那一条关系。
    fn peer_view(&self, peer: &Caller) -> Arc<View> {
        peer.team
            .space
            .nest
            .probe(KVirt::wrap(self.peer_at))
            .expect("the holder's relation")
    }

    /// manager 的一份私有空窗口：移交的目标端。
    fn seat(&self, lord: &Caller) -> usize {
        let handle = acquire_fresh(lord, 1);
        assert!(handle > 0, "a private seat: {handle}");
        handle as usize
    }

    /// 两端各自那份引用交还，再把那根根 Grant 交还（子树一起关）：
    /// 引用表与 Pie 都是调用方自己的责任，撤与关的两条路都在真实入口上走。
    fn close(&self, lord: &Caller, peer: &Caller) {
        release(lord, self.lord_handle);
        release(peer, self.peer_handle);
        assert_eq!(
            with_running(lord.task.clone(), || {
                lord.call(EnvCall::Pie(PieCall::Release {
                    token: self.root,
                    mode: env::ReleaseMode::Revoke,
                }))
            }),
            0,
            "the manager hands its root grant back"
        );
    }
}

/// 搭起那一对：manager 用私有来源铸根（每一页都物化出承载），持窗者的子分支
/// 在自己空间里 `open` 出压着同一批帧的关系。
fn across_spaces(lord: &Caller, peer: &Caller, pages: usize) -> AcrossSpaces {
    use env::{AcquireSource, Mark, Permission, PieToken};
    let source = acquire_fresh(lord, pages);
    assert!(source > 0, "the manager's source: {source}");
    for page in 0..pages {
        materialize_page(lord, source as usize, page);
    }
    let root = crate::entry::call::pole::create_pole(&lord.task, reference(source as usize), true)
        .expect("the manager's root grant");
    assert_eq!(
        lord.call(EnvCall::Memory(MemoryCall::Release {
            reference: reference(source as usize)
        })),
        0
    );
    let child = PieToken::mint(
        pend::accord(
            &lord.task,
            root,
            &Arc::downgrade(&peer.task),
            Permission::FETCH | Permission::STORE | Permission::VEST,
            Mark::NONE,
        )
        .expect("a child grant to the holder"),
    );
    let meta = pend::locate(&lord.task, root)
        .expect("the root pie")
        .mail()
        .pole()
        .expect("a pole");
    let rw = Access::READ | Access::WRITE;
    let (lord_at, _) =
        pole::open(&meta, root, &lord.team.space, rw).expect("the manager's relation");
    let (peer_at, _) =
        pole::open(&meta, child, &peer.team.space, rw).expect("the holder's relation");
    let lord_handle = lord.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: root,
            offset: 0,
            count: pages,
        },
        retention: PieToken::NONE,
    }));
    assert!(lord_handle > 0, "the manager's reference: {lord_handle}");
    let peer_handle = peer.call(EnvCall::Memory(MemoryCall::Acquire {
        source: AcquireSource::Source {
            token: child,
            offset: 0,
            count: pages,
        },
        retention: PieToken::NONE,
    }));
    assert!(peer_handle > 0, "the holder's reference: {peer_handle}");
    AcrossSpaces {
        root,
        child,
        meta,
        lord_handle: lord_handle as usize,
        peer_handle: peer_handle as usize,
        lord_at,
        peer_at,
    }
}

/// 夹具里的任务不是产品路径结束的：先按真实退出走一遍，再把最后那点收尸做全，
/// Team 与 Task 的持有链才不绕在那里。
///
/// 收尸那一手与生产 `reap` 同一条：`prune_tasks` 摘掉 Team 里那枚弱引用，名册里
/// 那一格的死 Task 也随 `prune_dead` 退场——少了它，每台 Caller 会在名册里钉住一份
/// `Arc` 的内脏（实测每台 512 B）。
///
/// 它管的是**断言之后**的那笔账：本夹具把 6 台 Caller 建在基线之前、终结在断言之后，
/// 故窗口里它们还活着（那一格本来就是 0），收尾不完整时涨的是**收尾之后**的逐类账；
/// 若有哪一形状真在**窗口内**结束一台任务（前一轮 59 那一形），它才是窗口里那一格的
/// 前提。
///
/// 这一步也是净额的一部分：退出之后引用、名下图与页槽都要归零。
fn retire(caller: &Caller) {
    if caller.task.dead() {
        return;
    }
    {
        let _commit = crate::core::work::commit();
        caller.task.state.lock().doom(None, env::ExitCause::Slay, 0);
    }
    pend::doom(&caller.task);
    caller.task.ident.team.prune_tasks();
    let _pruned = crate::core::work::room::scheduler::roster::prune_dead();
}

/// manager 把源页交出去：两个空间里那一页的 PTE 一起让出，而持窗者的关闭责任还在。
///
/// 清单里的那两条关系都要到消费者手里：调用者自己那条与**别人空间**里那条。
fn a_manager_takes_the_foreign_page_out(lord: &Caller, peer: &Caller) {
    use env::{MemoryFail, PoleCall};
    let pair = across_spaces(lord, peer, 2);
    let window = pair.lord_window(&lord);
    let pa = slot_at(&window, 0);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));

    *WAKE_TASK.lock() = Some(lord.task.clone());
    crate::entry::call::memory::arm_wake_probe(Some(pend_free_at_wake));
    WOKEN_KEYS.lock().clear();
    let seat = pair.seat(&lord);
    assert_eq!(
        lord.call(transfer_call(pair.lord_handle, 0, seat, 0)),
        0,
        "the page moves"
    );
    crate::entry::call::memory::arm_wake_probe(None);
    *WAKE_TASK.lock() = None;
    assert!(
        PEND_FREE_AT_WAKE.load(Ordering::Relaxed),
        "the wake happens after the caller's pend is released"
    );
    let mut woken = WOKEN_KEYS.lock().clone();
    woken.sort_unstable();
    let mut want = alloc::vec![
        (
            lord.team.space.nest.asid().get(),
            pair.lord_view(&lord).identity()
        ),
        (
            peer.team.space.nest.asid().get(),
            pair.peer_view(&peer).identity()
        ),
    ];
    want.sort_unstable();
    assert_eq!(
        woken, want,
        "both relations reach the consumer, the foreign one included"
    );

    // 两个空间里那一页的 PTE 都不在了；承载与整页内容到了目标槽。
    assert!(
        lord.team
            .space
            .nest
            .probe(KVirt::wrap(pair.lord_at))
            .is_some()
    );
    assert!(
        lord.team
            .space
            .nest
            .translate(KVirt::wrap(pair.lord_at))
            .is_none()
            && peer
                .team
                .space
                .nest
                .translate(KVirt::wrap(pair.peer_at))
                .is_none(),
        "every relation gives up that page, in both spaces"
    );
    assert_eq!(
        slots_of(&window_of(&lord, seat)),
        alloc::vec![(0, pa)],
        "the seat gets that carrier"
    );
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));
    assert!(tombstone(&window, 0), "the source page is a tombstone");

    // 持窗者那一枚**还欠着关图**：撤的是 PTE，不是它的关闭责任；它自己那一手关得掉。
    let peer_view = pair.peer_view(&peer);
    assert!(
        pair.meta.opened(&peer_view),
        "the holder still owes the close of its own window"
    );
    assert_eq!(
        with_running(peer.task.clone(), || {
            peer.call(EnvCall::Pole(PoleCall::Shut { token: pair.child }))
        }),
        0,
        "the holder closes its own window"
    );
    assert!(
        peer.team
            .space
            .nest
            .probe(KVirt::wrap(pair.peer_at))
            .is_none()
    );
    assert!(!pair.meta.opened(&peer_view));
    // 持窗者的引用与它的迁移都随那一枚一起走到尽头：不再是可用的一端。
    assert_eq!(
        peer.call(transfer_call(pair.peer_handle, 0, seat, 0)),
        MemoryFail::Denied.code(),
        "a closed branch is no longer an end"
    );
    pair.close(lord, peer);
    release(lord, seat);
    settle(lord, peer);
}

/// 持窗者先**退出**：它名下的图由退出路径收掉，manager 之后照旧搬得动那一页。
fn a_holder_leaving_gives_up_the_page(lord: &Caller, peer: &Caller) {
    let pair = across_spaces(lord, peer, 2);
    let window = pair.lord_window(&lord);
    let pa = slot_at(&window, 0);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));

    // 退出走真实的两步：状态先判死，再由 `pend::doom` 清表、cull 子树、收名下图。
    {
        let _operation = pole::PoleOperation::acquire(&pair.meta).expect("pole lease");
        {
            let _commit = crate::core::work::commit();
            peer.task.state.lock().doom(None, env::ExitCause::Slay, 0);
        }
        pend::doom(&peer.task);
    }
    assert!(peer.task.pend.pies.lock().is_empty());
    assert!(peer.task.pend.heirs.lock().is_empty());
    assert!(
        peer.team
            .space
            .nest
            .probe(KVirt::wrap(pair.peer_at))
            .is_none(),
        "the leaving holder's relation is gone"
    );
    assert_eq!(
        slots_of(&window),
        alloc::vec![(0, pa), (1, slot_at(&window, 1))],
        "leaving moves no carrier"
    );
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));

    // manager 自己那条关系照旧；现在搬那一页：压着它的只剩它自己那一条。
    let seat = pair.seat(&lord);
    assert_eq!(lord.call(transfer_call(pair.lord_handle, 0, seat, 0)), 0);
    assert_eq!(slots_of(&window_of(&lord, seat)), alloc::vec![(0, pa)]);
    assert!(
        lord.team
            .space
            .nest
            .translate(KVirt::wrap(pair.lord_at))
            .is_none()
    );
    assert_eq!(slots_of(&window_of(&lord, seat)), alloc::vec![(0, pa)]);
    assert!(tombstone(&window, 0));
    // 退出也把它的引用表一起清了：那边没有可交还的句柄，只剩这根根 Grant。
    assert!(peer.task.pend.references.get(pair.peer_handle).is_none());
    release(lord, pair.lord_handle);
    assert_eq!(
        with_running(lord.task.clone(), || {
            lord.call(EnvCall::Pie(PieCall::Release {
                token: pair.root,
                mode: env::ReleaseMode::Revoke,
            }))
        }),
        0
    );
    release(lord, seat);
    settle(lord, peer);
}

/// lord 撤掉那一枚：持窗者空间里那段关系一起消失，别的分支与承载都不动。
fn a_revoked_branch_stops_being_an_end(lord: &Caller, peer: &Caller) {
    use env::{MemoryFail, PieCall};
    let pair = across_spaces(lord, peer, 2);
    let window = pair.lord_window(&lord);
    let pa = slot_at(&window, 0);
    let second = slot_at(&window, 1);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));

    assert_eq!(
        with_running(lord.task.clone(), || {
            lord.call(EnvCall::Pie(PieCall::Revoke {
                dst: peer.task.ident.id,
                token: pair.child,
            }))
        }),
        0,
        "the lord revokes the branch it handed over"
    );
    assert!(pend::locate(&peer.task, pair.child).is_none());
    assert!(
        peer.team
            .space
            .nest
            .probe(KVirt::wrap(pair.peer_at))
            .is_none(),
        "the revoked branch's relation goes with it"
    );
    assert!(
        lord.team
            .space
            .nest
            .translate(KVirt::wrap(pair.lord_at))
            .is_some(),
        "the lord's own relation is untouched"
    );
    // 撤销不动承载：两页都还在原处，内容也没变。
    assert_eq!(slots_of(&window), alloc::vec![(0, pa), (1, second)]);
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));
    // 撤走的那条分支不再是可用的一端（解析不到窗口）。
    assert_eq!(
        peer.call(transfer_call(pair.peer_handle, 0, pair.peer_handle, 1)),
        MemoryFail::Denied.code()
    );
    // manager 自己那一手照走。
    let seat = pair.seat(&lord);
    assert_eq!(lord.call(transfer_call(pair.lord_handle, 0, seat, 0)), 0);
    assert_eq!(slots_of(&window_of(&lord, seat)), alloc::vec![(0, pa)]);
    assert_eq!(slots_of(&window), alloc::vec![(1, second)]);
    pair.close(lord, peer);
    release(lord, seat);
    settle(lord, peer);
}

/// 撤页确认在 manager 自己那个 Nest 上失败：两处都已经让出那一页（确认正是在撕下
/// 它之后才失败），两端页槽按原态回退，未结的同步责任留在失败的那个 Nest 里。
///
/// “确认失败发生在撕下任何 PTE 之前”那一读法（Nest 本来就脏）在核心层由
/// `a_failed_unpublish_prepare_keeps_the_mapping` 覆盖；这里只声称这一读法。
/// 处理次序就是关联链的次序。
fn a_failed_sync_stays_in_its_own_nest(lord: &Caller, peer: &Caller) {
    use crate::core::memory::nest::identity::fail_next_shootdown;
    use env::MemoryFail;
    let pair = across_spaces(lord, peer, 2);
    let window = pair.lord_window(&lord);
    let pa = slot_at(&window, 0);
    let second = slot_at(&window, 1);
    super::transfer::fill_pattern(PhysAddr::from_raw(pa));
    let seat = pair.seat(&lord);

    fail_next_shootdown(lord.team.space.nest.asid());
    assert_eq!(
        lord.call(transfer_call(pair.lord_handle, 0, seat, 0)),
        MemoryFail::Busy.code(),
        "an unconfirmed sync is retryable"
    );
    assert!(
        lord.team.space.nest.has_pending_invalidation(),
        "the unsettled sync stays with that Nest"
    );
    // 两处都把那一页让出去了——那一页的 PTE 不因后来的确认失败回填（这次确认就在
    // manager 自己那一页被撕下之后失败）。未结的失效责任留在那个 Nest 里。
    assert!(
        lord.team
            .space
            .nest
            .translate(KVirt::wrap(pair.lord_at))
            .is_none()
            && peer
                .team
                .space
                .nest
                .translate(KVirt::wrap(pair.peer_at))
                .is_none(),
        "a withdrawn page is not restored by a later failure"
    );
    // 两端页槽按原态还回：源两页都还是承载，目标那一格仍是 Fresh。
    assert_eq!(slots_of(&window), alloc::vec![(0, pa), (1, second)]);
    assert!(
        window_of(&lord, seat)
            .can_materialize(PageOffset::ZERO)
            .expect("the target's state"),
        "the target goes back to fresh"
    );
    assert!(slots_of(&window_of(&lord, seat)).is_empty());
    assert!(super::transfer::pattern_holds(PhysAddr::from_raw(pa)));

    // 结清那一笔之后重试：这一次真搬走，两个空间那一页都不再在。
    lord.team
        .space
        .nest
        .retry_invalidation()
        .expect("settle the injected failure");
    assert_eq!(lord.call(transfer_call(pair.lord_handle, 0, seat, 0)), 0);
    assert_eq!(slots_of(&window_of(&lord, seat)), alloc::vec![(0, pa)]);
    assert!(
        lord.team
            .space
            .nest
            .translate(KVirt::wrap(pair.lord_at))
            .is_none()
    );
    assert!(
        peer.team
            .space
            .nest
            .translate(KVirt::wrap(pair.peer_at))
            .is_none()
    );
    assert!(tombstone(&window, 0));
    assert_eq!(slots_of(&window), alloc::vec![(1, second)]);
    pair.close(lord, peer);
    release(lord, seat);
    settle(lord, peer);
}

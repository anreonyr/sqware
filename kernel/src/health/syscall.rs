#![cfg(debug_assertions)]

use alloc::sync::Arc;
use env::{ControlCall, DebugCall, EnvCall, MemoryCall, ProgramKind, RoomCall, UnitCall, VirtAddr};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::{addr::VirtAddr as KVirt, entry::PteFlags, mode};
use crate::runtime::diagnose::frame::StackReader;
use crate::runtime::switcher::{
    context::{Gprs, TrapContext},
    envcall,
};
use crate::work::mail::{hole, pole};
use crate::work::unit::gate::{self, Need, Permission};
use crate::work::unit::space::{SpaceBuilder, window::HeapWindow};
use crate::work::unit::task::{Task, TaskTag};
use crate::work::unit::team::{Team, TeamBuilder};

struct Caller {
    team: Arc<Team>,
    task: Arc<Task>,
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
        space.with(|inner| inner.dynamic(0x4000_0000));
        let team = TeamBuilder::new(space).spawn().unwrap();
        let task = team.task().hold().unwrap();
        Self { team, task }
    }

    fn raw(&self, slot: usize, regs: [usize; 6]) -> isize {
        // SAFETY: the task is held and never scheduled during this test.
        let frame =
            unsafe { &mut *(self.task.ident.frame.pa.unwrap().as_usize() as *mut TrapContext) };
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

    fn allocate(&self, size: usize) -> usize {
        let at = self.call(EnvCall::Memory(MemoryCall::Allocate { size }));
        assert!(at > 0);
        at as usize
    }
}

impl Drop for Caller {
    fn drop(&mut self) {
        self.team.release_held(&self.task);
        self.team.prune_tasks(&self.task);
    }
}

pub fn abi_and_privilege() {
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    use env::wire::{Decode, Wire};
    let user = Caller::new(false);
    assert!(user.raw(3usize << 32, [0; 6]) < 0);
    assert!(user.raw((2usize << 32) | 99, [0; 6]) < 0);
    assert!(user.raw((1usize << 32) | 5, [2, 0, 0, 0, 0, 0]) < 0);
    assert!(user.raw((5usize << 32) | 3, [0, 2, 0, 0, 0, 0]) < 0);
    assert!(user.raw((7usize << 32) | 6, [0, 0, usize::MAX, 0, 0, 0]) < 0);
    assert!(matches!(
        bool::unpack(&[2, 0, 0, 0, 0, 0], &mut 0),
        Err(Decode::Invalid)
    ));
    let build = |kind| EnvCall::Unit(UnitCall::Build { kind });
    let heirs = user.task.heir_count();
    assert_eq!(
        user.call(build(ProgramKind::Supervisor)),
        env::UnitFail::Denied.code()
    );
    assert_eq!(user.task.heir_count(), heirs);
    assert_eq!(user.call(build(ProgramKind::User)), env::UnitFail::Denied.code());
    let ordinary = crate::work::mail::nole::NoleMeta::new(user.task.ident.id);
    user.task.pies.lock().push(gate::boxed(gate::new_pie::<gate::Nole>(
        ordinary, env::Mark::NONE, Permission::FETCH | Permission::VEST, None,
    )).expect("pie allocation"));
    assert_eq!(user.call(build(ProgramKind::User)), env::UnitFail::Denied.code());
    assert_eq!(
        user.call(EnvCall::Room(RoomCall::Doom {
            task: user.task.ident.id
        })),
        env::RoomFail::Denied.code()
    );
    assert_eq!(user.task.tag(), TaskTag::Held);
    let mut registry = crate::resource::Registry::default();
    crate::runtime::switcher::envcall::resources::register(&mut registry).unwrap();
    registry.freeze().unwrap().grant(&user.task).unwrap();
    assert_eq!(user.call(build(ProgramKind::Supervisor)), env::UnitFail::Denied.code());
    assert!(user.call(build(ProgramKind::User)) > 0);
    assert_eq!(user.call(EnvCall::Room(RoomCall::Doom {
        task: env::TaskId::new(usize::MAX),
    })), env::RoomFail::Dead.code());
    let supervisor = Caller::new(true);
    assert_eq!(supervisor.call(EnvCall::Room(RoomCall::Doom {
        task: user.task.ident.id,
    })), env::RoomFail::Denied.code());
    assert_eq!(supervisor.call(build(ProgramKind::User)), env::UnitFail::Denied.code());
    let mut registry = crate::resource::Registry::default();
    crate::runtime::switcher::envcall::resources::register(&mut registry).unwrap();
    registry.freeze().unwrap().grant(&supervisor.task).unwrap();
    for kind in [ProgramKind::User, ProgramKind::Supervisor] {
        assert!(supervisor.call(EnvCall::Unit(UnitCall::Build { kind })) > 0);
    }
    assert_eq!(
        supervisor.call(EnvCall::Room(RoomCall::Doom {
            task: env::TaskId::new(usize::MAX)
        })),
        env::RoomFail::Dead.code()
    );
}

pub fn memory() {
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    let user = Caller::new(false);
    let zero = user.allocate(0);
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Deallocate {
            addr: VirtAddr::new(zero),
            size: 0
        })),
        0
    );
    let at = user.allocate(2 * PAGE_SIZE);
    let before = user.team.space.translate(KVirt::wrap(at)).unwrap().1.bits();
    for flags in [1, 16, 32, 64, 128, 256, 1u64 << 63, 0, 4] {
        assert!(
            user.call(EnvCall::Memory(MemoryCall::Mprotect {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(at),
                size: PAGE_SIZE,
                flags
            })) < 0
        );
        assert_eq!(
            user.team.space.translate(KVirt::wrap(at)).unwrap().1.bits(),
            before
        );
    }
    for flags in [2, 8, 10, 6] {
        assert_eq!(
            user.call(EnvCall::Memory(MemoryCall::Mprotect {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(at),
                size: PAGE_SIZE,
                flags
            })),
            0
        );
        let actual = user.team.space.translate(KVirt::wrap(at)).unwrap().1.bits();
        assert_eq!(actual & !14, before & !14);
        assert_eq!(actual & 14, flags);
    }
    for internal in [
        user.task.ident.frame.va.as_usize(),
        crate::layout::TRAMPOLINE.as_usize(),
        mode::lower().as_usize(),
        at + (mode::upper().as_usize() << 1),
    ] {
        for call in [
            MemoryCall::Mprotect {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
                flags: 6,
            },
            MemoryCall::Munmap {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
            },
            MemoryCall::Deallocate {
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
            },
            MemoryCall::Mmap {
                team: env::TeamId::new(0),
                backing: env::PieToken::NONE,
                offset: 0,
                flags: 6,
                at: VirtAddr::new(internal),
                size: PAGE_SIZE,
            },
        ] {
            assert!(user.call(EnvCall::Memory(call)) < 0);
        }
    }
    for size in [usize::MAX, usize::MAX - PAGE_SIZE + 1] {
        for call in [
            MemoryCall::Allocate { size },
            MemoryCall::Deallocate {
                addr: VirtAddr::new(at),
                size,
            },
            MemoryCall::Mmap {
                team: env::TeamId::new(0),
                backing: env::PieToken::NONE,
                offset: 0,
                flags: 6,
                at: VirtAddr::new(0),
                size,
            },
            MemoryCall::Munmap {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(at),
                size,
            },
            MemoryCall::Mprotect {
                team: env::TeamId::new(0),
                addr: VirtAddr::new(at),
                size,
                flags: 6,
            },
        ] {
            assert!(user.call(EnvCall::Memory(call)) < 0);
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
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Deallocate {
            addr: VirtAddr::new(at),
            size: 2 * PAGE_SIZE
        })),
        0
    );
    assert!(user.team.space.translate(KVirt::wrap(at)).is_none());

    let fixed = 0x4004_0000;
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Mmap {
            team: env::TeamId::new(0),
            backing: env::PieToken::NONE,
            offset: 0,
            flags: 6,
            at: VirtAddr::new(fixed),
            size: 3 * PAGE_SIZE
        })),
        fixed as isize
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Mprotect {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(fixed + PAGE_SIZE),
            size: PAGE_SIZE,
            flags: 2
        })),
        0
    );
    for (page, writable) in [
        (fixed, true),
        (fixed + PAGE_SIZE, false),
        (fixed + 2 * PAGE_SIZE, true),
    ] {
        user.team
            .space
            .materialize(KVirt::wrap(page), PAGE_SIZE)
            .unwrap();
        assert_eq!(
            user.team
                .space
                .translate(KVirt::wrap(page))
                .unwrap()
                .1
                .contains(PteFlags::W),
            writable
        );
    }
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Munmap {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(fixed + PAGE_SIZE),
            size: PAGE_SIZE
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Munmap {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(fixed),
            size: 3 * PAGE_SIZE
        })),
        0
    );
    assert!(user.team.space.translate(KVirt::wrap(fixed)).is_none());
}

pub fn pointers() {
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    let user = Caller::new(false);
    let at = user.allocate(PAGE_SIZE);
    assert!(user.team.space.copy_out(b"readable", at));
    let mut bytes = [0xa5; 8];
    for (va, len) in [
        (0, 8),
        (user.task.ident.frame.va.as_usize(), 8),
        (crate::layout::TRAMPOLINE.as_usize(), 8),
        (at + PAGE_SIZE - 4, 8),
        (at, usize::MAX),
        (usize::MAX - 3, 8),
    ] {
        assert!(!user.team.space.validate_read(va, len));
        assert!(!user.team.space.validate_write(va, len));
        assert!(
            user.call(EnvCall::Debug(DebugCall::Put {
                buf: VirtAddr::new(va),
                len
            })) < 0
        );
        if !user.team.space.validate_read(va, size_of::<usize>()) {
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
    assert!(!user.team.space.copy_in(&mut bytes, at + PAGE_SIZE - 4));
    assert_eq!(bytes, [0xa5; 8]);
    assert!(user.team.space.copy_in(&mut [], 0));
    assert!(user.team.space.copy_out(&[], 0));
    assert_eq!(
        user.call(EnvCall::Debug(DebugCall::Put {
            buf: VirtAddr::new(0),
            len: 0
        })),
        env::DebugFail::Denied.code()
    );
    assert!(
        user.call(EnvCall::Debug(DebugCall::Get {
            buf: VirtAddr::new(user.task.ident.frame.va.as_usize()),
            len: 8
        })) < 0
    );
    assert!(
        user.call(EnvCall::Control(ControlCall::Backtrace {
            buf: user.task.ident.frame.va.as_usize(),
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
    assert!(reader.word(user.task.ident.frame.va.as_usize()).is_none());
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Mprotect {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(at),
            size: PAGE_SIZE,
            flags: 8
        })),
        0
    );
    assert!(!user.team.space.copy_in(&mut bytes, at));
    assert!(
        user.call(EnvCall::Debug(DebugCall::Put {
            buf: VirtAddr::new(at),
            len: 8
        })) < 0
    );
    assert!(user.team.space.instruction_byte(at).is_some());
    let foreign = Caller::new(false);
    assert!(!foreign.team.space.validate_read(at, 8));
}

pub fn capability() {
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    let user = Caller::new(false);
    let foreign = Caller::new(false);
    let meta = hole::meta(user.task.ident.id);
    let pie = gate::new_pie::<gate::Hole>(meta.clone(), env::Mark::NONE, Permission::FETCH, None);
    let token = pie.token;
    user.task.pies.lock().push(gate::boxed(pie).expect("pie allocation"));
    assert!(gate::accede::<env::PieFail>(&user.task, token, Need::Fetch).is_ok());
    assert!(matches!(
        gate::accede::<env::PieFail>(&user.task, token, Need::Store),
        Err(env::PieFail::Denied)
    ));
    assert!(matches!(
        gate::accede::<env::PieFail>(&foreign.task, token, Need::Fetch),
        Err(env::PieFail::Denied)
    ));
    assert!(matches!(
        gate::accede::<env::PieFail>(&user.task, env::PieToken::mint(usize::MAX), Need::Fetch),
        Err(env::PieFail::Denied)
    ));
    hole::seal(&meta);
    assert!(matches!(
        gate::accede::<env::PieFail>(&user.task, token, Need::Fetch),
        Err(env::PieFail::Dead)
    ));
    let pole = pole::meta(PAGE_SIZE, user.task.ident.id).unwrap();
    let pie: gate::Pie<gate::Pole> =
        gate::new_pie(pole.clone(), env::Mark::NONE, Permission::FETCH, None);
    let token = pie.token;
    let flags = user
        .team
        .space
        .pte_policy(PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D);
    let (at, size) = pole::open(&pole, token, &user.team.space, flags).unwrap();
    for call in [
        MemoryCall::Deallocate {
            addr: VirtAddr::new(at),
            size,
        },
        MemoryCall::Munmap {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(at),
            size,
        },
        MemoryCall::Mprotect {
            team: env::TeamId::new(0),
            addr: VirtAddr::new(at),
            size,
            flags: 6,
        },
    ] {
        assert!(user.call(EnvCall::Memory(call)) < 0);
    }
    assert!(user.team.space.validate_read(at, size));
    pole::shut(&pole, token).unwrap();
    assert!(!user.team.space.validate_read(at, size));
    let _ = HeapWindow::allocate(&user.team.space, PAGE_SIZE).unwrap();
}


pub fn resource_registration() {
    use crate::resource::Registry;
    use crate::runtime::switcher::{envcall::resources as calls, trap::resources as traps};
    use env::{Call, Mark, Name, PieFail, PieToken, Trap};
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    let system = Caller::new(true);
    let service = Caller::new(false);
    let other = Caller::new(false);
    let mut registry = Registry::default();
    calls::register(&mut registry).unwrap();
    traps::register(&mut registry).unwrap();
    let root = || gate::boxed(gate::new_pie::<gate::Nole>(
        calls::get().build.clone(), Mark::NONE, Permission::FETCH | Permission::VEST, None,
    )).expect("pie allocation");
    let duplicate = root();
    let token = duplicate.token();
    let error = registry.register(Name::Call(Call::Build), duplicate).unwrap_err();
    assert_eq!(error.reason, PieFail::Denied);
    assert_eq!(error.value.1.token(), token);
    assert!(registry.register(Name::Trap(Trap::PageFault), root()).is_err());
    let entries = registry.freeze().unwrap().grant(&system.task).unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(env::Entry::valid));
    let token = entries.iter().find(|e| e.name() == Name::Call(Call::Build)).unwrap().token();
    assert!(gate::allows(&system.task, &calls::get().build, Need::Fetch));
    let token = PieToken::mint(gate::accord(
        &system.task, token, &Arc::downgrade(&service.task), Permission::FETCH, Mark::NONE,
    ).unwrap());
    assert!(gate::allows(&service.task, &calls::get().build, Need::Fetch));
    assert!(service.call(EnvCall::Unit(UnitCall::Build { kind: ProgramKind::User })) > 0);
    assert!(matches!(gate::accord(
        &service.task, token, &Arc::downgrade(&other.task), Permission::FETCH, Mark::NONE,
    ), Err(PieFail::Denied)));
    gate::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    assert!(!gate::allows(&service.task, &calls::get().build, Need::Fetch));
    assert_eq!(service.call(EnvCall::Unit(UnitCall::Build { kind: ProgramKind::User })), env::UnitFail::Denied.code());
    let doom = entries.iter().find(|e| e.name() == Name::Call(Call::Doom)).unwrap().token();
    let call = EnvCall::Room(RoomCall::Doom { task: env::TaskId::new(usize::MAX) });
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    let token = PieToken::mint(gate::accord(
        &system.task, doom, &Arc::downgrade(&service.task), Permission::VEST, Mark::NONE,
    ).unwrap());
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    gate::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    let token = PieToken::mint(gate::accord(
        &system.task, doom, &Arc::downgrade(&service.task), Permission::FETCH, Mark::NONE,
    ).unwrap());
    assert!(gate::allows(&service.task, &calls::get().doom, Need::Fetch));
    let victim = Caller::new(false);
    assert_eq!(service.call(EnvCall::Room(RoomCall::Doom {
        task: victim.task.ident.id,
    })), 0);
    assert!(matches!(victim.task.tag(), TaskTag::Doomed | TaskTag::Reaped));
    assert_eq!(service.call(call), env::RoomFail::Dead.code());
    assert!(matches!(gate::accord(
        &service.task, token, &Arc::downgrade(&other.task), Permission::FETCH, Mark::NONE,
    ), Err(PieFail::Denied)));
    gate::revoke(&system.task, &Arc::downgrade(&service.task), token).unwrap();
    assert_eq!(service.call(call), env::RoomFail::Denied.code());
    let mut registry = Registry::default();
    let dead = crate::work::mail::nole::NoleMeta::new(env::TaskId::new(0));
    let invalid = gate::boxed(gate::new_pie::<gate::Nole>(
        dead.clone(), Mark::NONE, Permission::FETCH | Permission::VEST, None,
    )).expect("pie allocation");
    registry.register(Name::Call(Call::Build), invalid).unwrap();
    crate::work::mail::nole::seal(&dead);
    let frozen = match registry.freeze() { Err(e) => e, Ok(_) => panic!("accepted dead root") };
    assert_eq!(frozen.reason, PieFail::Denied);

    let dead = crate::work::mail::nole::NoleMeta::new(env::TaskId::new(0));
    let mut registry = Registry::default();
    let invalid = gate::boxed(gate::new_pie::<gate::Nole>(
        dead.clone(), Mark::NONE, Permission::FETCH | Permission::VEST, None,
    )).expect("pie allocation");
    registry.register(Name::Call(Call::Build), invalid).unwrap();
    let frozen = registry.freeze().unwrap();
    crate::work::mail::nole::seal(&dead);
    let before = other.task.pies.lock().len();
    let error = frozen.grant(&other.task).unwrap_err();
    assert_eq!(error.reason, PieFail::Denied);
    assert_eq!(error.value.len(), 1);
    assert_eq!(other.task.pies.lock().len(), before);
}

/// Native checks use real task locks, permits, mappings and the debug lock checker.
pub fn transfer_relations() {
    use alloc::vec::Vec;
    use env::{Mark, PieFail, PieToken};
    crate::work::room::scheduler::boot::init().expect("scheduler init");
    let a = Caller::new(false);
    let b = Caller::new(false);
    let c = Caller::new(false);
    let permission = Permission::FETCH | Permission::STORE | Permission::VEST;
    let root = |task: &Arc<Task>, permission| {
        let pie = gate::boxed(gate::new_pie::<gate::Nole>(
            crate::work::mail::nole::NoleMeta::new(task.ident.id), Mark::NONE, permission, None,
        )).expect("pie allocation");
        let token = pie.token();
        gate::insert(task, pie).unwrap();
        token
    };
    let grant = |task: &Arc<Task>, token, target: &Arc<Task>, permission| {
        PieToken::mint(gate::accord(task, token, &Arc::downgrade(target), permission, Mark::NONE).unwrap())
    };
    let token = root(&a.task, permission);
    let ab = grant(&a.task, token, &b.task, permission);
    let bc = grant(&b.task, ab, &c.task, permission);
    assert_eq!(gate::vestor(&c.task, bc), Some(b.task.ident.id));
    gate::forget(&b.task, ab).unwrap();
    assert_eq!(gate::vestor(&c.task, bc), Some(a.task.ident.id));
    assert!(b.task.pies.lock().is_empty());
    assert!(b.task.heirs.lock().is_empty());
    assert_eq!(gate::release(&a.task, token), Ok(2));
    assert!(c.task.pies.lock().is_empty());

    let token = root(&a.task, permission);
    let child = grant(&a.task, token, &a.task, permission);
    assert_eq!(gate::vestor(&a.task, child), Some(a.task.ident.id));
    assert_eq!(gate::release(&a.task, token), Ok(2));

    let sole = permission | Permission::ONLY;
    let token = root(&a.task, sole);
    let ab = grant(&a.task, token, &b.task, sole);
    let old = gate::locate(&a.task, token).unwrap().heir().copied().unwrap();
    gate::revoke(&a.task, &Arc::downgrade(&b.task), ab).unwrap();
    let next = grant(&a.task, token, &b.task, sole);
    assert!(!gate::clear_heir(&a.task, token, old));
    assert_eq!(gate::locate(&a.task, token).unwrap().heir().unwrap().token, next);
    gate::release(&a.task, token).unwrap();

    // More simultaneous gates than the previous fixed eight-lock debug limit.
    let children: Vec<_> = (0..16).map(|_| Caller::new(false)).collect();
    let token = root(&a.task, permission);
    for child in &children { grant(&a.task, token, &child.task, permission); }
    assert_eq!(gate::release(&a.task, token), Ok(17));
    assert!(children.iter().all(|child| child.task.pies.lock().is_empty()));

    // Exit also withdraws borrowed memory while a backing operation is busy.
    let d = Caller::new(false);
    let meta = pole::meta(PAGE_SIZE, a.task.ident.id).unwrap();
    let pie = gate::boxed(gate::try_new_pie::<gate::Pole>(meta.clone(), Mark::NONE, permission, None).unwrap()).expect("pie allocation");
    let memory = pie.token();
    gate::insert(&a.task, pie).unwrap();
    let ad = grant(&a.task, memory, &d.task, permission);
    grant(&d.task, ad, &c.task, permission);
    {
        let _operation = meta.backing().operation().unwrap();
        gate::doom(&d.task);
    }
    assert!(d.task.pies.lock().is_empty());
    assert!(c.task.pies.lock().is_empty());

    let token = root(&a.task, permission);
    let ab = grant(&a.task, token, &b.task, permission);
    grant(&b.task, ab, &c.task, permission);
    gate::doom(&b.task);
    assert!(b.task.pies.lock().is_empty());
    assert!(c.task.pies.lock().is_empty());
    assert!(a.task.heirs.lock().is_empty());
    assert_eq!(gate::accord(&a.task, token, &Arc::downgrade(&b.task), permission, Mark::NONE), Err(PieFail::Dead));
}

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
use crate::work::unit::gate::{self, AnyPie, Need, Permission};
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

    fn image(&self) -> usize {
        let mut elf = [0u8; 128];
        elf[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&243u16.to_le_bytes());
        elf[24..32].copy_from_slice(&0x10000u64.to_le_bytes());
        elf[32..40].copy_from_slice(&64u64.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        elf[64..68].copy_from_slice(&1u32.to_le_bytes());
        elf[68..72].copy_from_slice(&5u32.to_le_bytes());
        elf[80..88].copy_from_slice(&0x10000u64.to_le_bytes());
        elf[96..104].copy_from_slice(&128u64.to_le_bytes());
        elf[104..112].copy_from_slice(&(PAGE_SIZE as u64).to_le_bytes());
        let at = self.allocate(PAGE_SIZE);
        assert!(self.team.space.copy_out(&elf, at));
        at
    }
}

impl Drop for Caller {
    fn drop(&mut self) {
        self.team.release_held(&self.task);
        self.team.prune_tasks(&self.task);
    }
}

pub fn abi_and_privilege() {
    crate::work::room::scheduler::boot::init();
    use env::wire::{Decode, Wire};
    let user = Caller::new(false);
    assert!(user.raw(3usize << 32, [0; 6]) < 0);
    assert!(user.raw((2usize << 32) | 99, [0; 6]) < 0);
    assert!(user.raw((1usize << 32) | 5, [0, 0, 2, 0, 0, 0]) < 0);
    assert!(user.raw((5usize << 32) | 3, [0, 2, 0, 0, 0, 0]) < 0);
    assert!(user.raw((7usize << 32) | 6, [0, 0, usize::MAX, 0, 0, 0]) < 0);
    assert!(matches!(
        bool::unpack(&[2, 0, 0, 0, 0, 0], &mut 0),
        Err(Decode::Invalid)
    ));
    let elf = user.image();
    let build = |kind| {
        EnvCall::Unit(UnitCall::Build {
            elf: VirtAddr::new(elf),
            len: 128,
            kind,
        })
    };
    let heirs = user.task.heir_count();
    assert_eq!(
        user.call(build(ProgramKind::Supervisor)),
        env::UnitFail::Denied.code()
    );
    assert_eq!(user.task.heir_count(), heirs);
    assert!(user.call(build(ProgramKind::User)) > 0);
    for (address, size) in [
        (crate::layout::TEAM_FRAME_BASE.as_usize(), PAGE_SIZE),
        (usize::MAX - PAGE_SIZE + 1, PAGE_SIZE),
        (mode::upper().as_usize() - PAGE_SIZE, PAGE_SIZE + 1),
    ] {
        assert!(
            user.team
                .space
                .copy_out(&(address as u64).to_le_bytes(), elf + 80)
        );
        assert!(
            user.team
                .space
                .copy_out(&(size as u64).to_le_bytes(), elf + 104)
        );
        assert_eq!(
            user.call(build(ProgramKind::User)),
            env::UnitFail::BadImage.code()
        );
    }
    assert_eq!(
        user.call(EnvCall::Room(RoomCall::Doom {
            task: user.task.ident.id
        })),
        env::RoomFail::Denied.code()
    );
    assert_eq!(user.task.tag(), TaskTag::Held);
    let supervisor = Caller::new(true);
    let elf = supervisor.image();
    for kind in [ProgramKind::User, ProgramKind::Supervisor] {
        assert!(
            supervisor.call(EnvCall::Unit(UnitCall::Build {
                elf: VirtAddr::new(elf),
                len: 128,
                kind
            })) > 0
        );
    }
    assert_eq!(
        supervisor.call(EnvCall::Room(RoomCall::Doom {
            task: env::TaskId::new(usize::MAX)
        })),
        env::RoomFail::Dead.code()
    );
}

pub fn memory() {
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
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
                flags: 6,
            },
            MemoryCall::Munmap {
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
            },
            MemoryCall::Deallocate {
                addr: VirtAddr::new(internal),
                size: PAGE_SIZE,
            },
            MemoryCall::Mmap {
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
                at: VirtAddr::new(0),
                size,
            },
            MemoryCall::Munmap {
                addr: VirtAddr::new(at),
                size,
            },
            MemoryCall::Mprotect {
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
            at: VirtAddr::new(fixed),
            size: 3 * PAGE_SIZE
        })),
        fixed as isize
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Mprotect {
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
            addr: VirtAddr::new(fixed + PAGE_SIZE),
            size: PAGE_SIZE
        })),
        0
    );
    assert_eq!(
        user.call(EnvCall::Memory(MemoryCall::Munmap {
            addr: VirtAddr::new(fixed),
            size: 3 * PAGE_SIZE
        })),
        0
    );
    assert!(user.team.space.translate(KVirt::wrap(fixed)).is_none());
}

pub fn pointers() {
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
        assert!(
            user.call(EnvCall::Unit(UnitCall::Build {
                elf: VirtAddr::new(va),
                len,
                kind: ProgramKind::User
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
    let user = Caller::new(false);
    let foreign = Caller::new(false);
    let meta = hole::meta(user.task.ident.id);
    let pie = gate::new_pie(meta.clone(), env::Mark::NONE, Permission::FETCH, None);
    let token = pie.token;
    user.task.pies.lock().push(AnyPie::Hole(pie));
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
            addr: VirtAddr::new(at),
            size,
        },
        MemoryCall::Mprotect {
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

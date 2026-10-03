//! Observe a system team's failure from an independent parent team.
use crate::system::life::{Phase, Status};
use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use env::{Mark, Permission, TaskId, TeamId, Wait};
use protocol::communication::session::establish;
use runtime::env::{
    mail::{self, HolePie, PolePie},
    room, unit,
};

const REPORT: Mark = Mark::of("system-fault-report");
const BOOT: Mark = Mark::of("system-fault-boot");
const IMAGE: Mark = Mark::of("system-fault-child-image");

pub fn acceptance() {
    let accounts = crate::boot::Accounts::take().unwrap();
    let catalog = crate::boot::Catalog::of_boot(&accounts).unwrap();
    let victim = catalog.find("system-fault-unit").unwrap();
    let child = catalog.find("system-child").unwrap();
    let payload = PolePie::unseal(child.elf.len().div_ceil(runtime::PAGE_SIZE) * runtime::PAGE_SIZE).unwrap();
    let (at, _) = payload.open().unwrap();
    // SAFETY: the owned writable Pole covers the complete ELF payload.
    unsafe { core::ptr::copy_nonoverlapping(child.elf.as_ptr(), at as *mut u8, child.elf.len()); }
    let report = mail::unseal_hole(REPORT).unwrap();
    let boot = mail::unseal_hole(BOOT).unwrap();
    for mode in 0..3 {
        let image = runtime::core::loader::build(victim.elf, victim.kind).unwrap();
        let team = image.team();
        let task = image.spawn(&[mode, child.elf.len(), unit::self_id().get()], 0).unwrap();
        runtime::core::res::port::ship(&HolePie::from_token(report), task,
            env::Access::STORE, env::Policy::NONE).unwrap();
        runtime::core::res::port::ship(&HolePie::from_token(boot), task,
            env::Access::FETCH, env::Policy::NONE).unwrap();
        let token = env::pie::accord(payload.token(), task, Permission::FETCH, IMAGE).unwrap();
        HolePie::from_token(boot).push(&token.to_bytes(), Wait::AtMost(5000)).unwrap();
        unit::embark(task).unwrap();
        let mut bytes = [0; 40];
        let (n, from) = HolePie::from_token(report).pull(&mut bytes, Wait::AtMost(5000)).unwrap();
        assert_eq!(from, task);
        assert_eq!(n, bytes.len());
        let ids: alloc::vec::Vec<usize> = bytes.chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize).collect();
        assert_eq!(ids[0], task.get());
        let until = runtime::env::chrono::clock() + 5_000_000_000;
        for id in &ids[..4] {
            while !unit::join(TaskId::new(*id), Wait::POLL).unwrap_or(true) {
                assert!(runtime::env::chrono::clock() < until, "system-fault: task survived team failure");
                room::sleep(core::time::Duration::from_millis(1)).unwrap();
            }
        }
        unit::oust(team).unwrap();
        protocol::debug::put(&alloc::format!("system-fault: role={mode}; three tasks and descendant reclaimed"));
    }
    let _ = mail::seal(boot);
    let _ = mail::release(boot);
    let _ = mail::seal(report);
    let _ = mail::release(report);
}

pub fn unit() {
    let args = runtime::core::task::args::args();
    let mode = args[0];
    let sire = TaskId::new(args[2]);
    let boot = establish::find(sire, BOOT).unwrap();
    let mut bytes = [0; 8];
    let (n, from) = HolePie::from_token(boot).pull(&mut bytes, Wait::AtMost(5000)).unwrap();
    assert_eq!((n, from), (bytes.len(), sire));
    let payload = env::PieToken::from_bytes(&bytes).unwrap();
    let dock = runtime::core::res::dock::Dock::open(PolePie::from_token(payload)).unwrap();
    // SAFETY: the parent supplied an immutable ELF payload of args[1] bytes.
    let elf = unsafe { core::slice::from_raw_parts(dock.view().base() as *const u8, args[1]) };
    let child = runtime::core::loader::build(elf, env::ProgramKind::User).unwrap();
    let child_team = child.team();
    let descendant = child.spawn(&[], 0).unwrap();
    unit::embark(descendant).unwrap();
    let status = Arc::new(Status {
        control: unit::self_id(), operator: AtomicUsize::new(0), identity: AtomicUsize::new(0),
        phase: AtomicU8::new(Phase::Running as u8),
    });
    for (role, slot) in [(1, &status.operator), (2, &status.identity)] {
        let state = status.clone();
        let body: Box<dyn FnOnce(usize) + Send> = Box::new(move |_| {
            if mode == role {
                room::exit(env::EXIT_OK, Some("system-fault: injected task exit"));
            }
            let success = if role == 1 {
                crate::system::operator::serve::run::serve(state.clone()).is_ok()
            } else {
                crate::system::identity::serve::run::serve(
                    state.clone(),
                    crate::system::identity::serve::revision::Epoch::new(),
                    crate::system::identity::serve::revision::Changed(
                        mail::NolePie::unseal().unwrap(),
                    ),
                )
                .is_ok()
            };
            if !success {
                let _ = room::doom(unit::self_id());
            }
        });
        let ptr = Box::into_raw(Box::new(body));
        let task = unit::spawn(TeamId::new(0), runtime::core::task::join::trampoline as *const () as usize,
            &[ptr as usize], 0).unwrap();
        slot.store(task.get(), Ordering::Release);
    }
    let operator = status.operator.load(Ordering::Acquire);
    let identity = status.identity.load(Ordering::Acquire);
    let report = establish::find(sire, REPORT).unwrap();
    let mut bytes = [0; 40];
    for (slot, id) in bytes.chunks_exact_mut(8).zip([
        status.control.get(), operator, identity, descendant.get(), child_team.get(),
    ]) { slot.copy_from_slice(&(id as u64).to_le_bytes()); }
    HolePie::from_token(report).push(&bytes, Wait::AtMost(5000)).unwrap();
    unit::embark(TaskId::new(operator)).unwrap();
    unit::embark(TaskId::new(identity)).unwrap();
    if mode == 0 { return; }
    loop {
        if [operator, identity].iter().any(|id| unit::join(TaskId::new(*id), Wait::POLL).unwrap_or(true)) {
            let _ = room::doom(unit::self_id());
        }
        room::sleep(core::time::Duration::from_millis(10)).unwrap();
    }
}

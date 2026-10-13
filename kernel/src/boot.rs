pub(crate) mod error;

pub use error::{BootError, MapOperation, ResourceOperation, fail};

use core::arch::global_asm;

use riscv::register::satp;

use crate::hart::{self, HartId};
use crate::layout::{ROOT_STACK_CANARY, root_stack_base};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::mode;
use crate::platform::machine;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::restore;
use crate::runtime::switcher::trap::{arm_hart, boot_stack};
use crate::work::room::scheduler;
use crate::work::unit::team::kernel;

global_asm!(
    ".section .text.boot",
    ".align 2",
    ".globl _boot_entry",
    "_boot_entry:",
    "    la   t0, PER_HART",
    "    slli t1, a0, 6",
    "    add  tp, t0, t1",
    "    csrc sstatus, 2",
    "    mv   sp, a1",
    "    call boot_main",
);

unsafe extern "C" {
    static _boot_entry: u8;
}

pub fn init() -> Result<(), BootError> {
    scheduler::boot::init().map_err(BootError::Scheduler)?;
    register_runtime_hooks();

    #[cfg(debug_assertions)]
    crate::lock::init_depend(hart::hart_count()).map_err(BootError::Dependencies)?;

    #[cfg(debug_assertions)]
    crate::health::run();

    if crate::testing() && machine::info().initrd().is_none() {
        return Err(BootError::MissingImage);
    }

    if let Some(entry) = spawn_entry()? {
        crate::work::room::scheduler::core::beacon_arm(&entry);
    }

    crate::work::room::conductor::rooted();

    boot_harts()?;

    #[cfg(debug_assertions)]
    if crate::testing() {
        crate::runtime::diagnose::ipi::run("early", None);
        crate::runtime::diagnose::ipi::start_delayed();
    }

    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );
    Ok(())
}

pub fn run() -> ! {
    restore(scheduler::trap::run())
}

fn register_runtime_hooks() {
    use crate::work::room::conductor;
    use crate::work::room::messenger;

    /// 退场那两位的签名：**收 `&Arc<Task>`**（号那一格逼得封印去问清册快照，而快照会分配，
    /// 备不出容量就空表——故直接收 `Arc`）。
    static EXIT_HOOKS: &[fn(&alloc::sync::Arc<crate::work::unit::task::Task>)] = &[
        crate::work::room::messenger::doom,
        crate::work::unit::gate::doom,
    ];
    messenger::hook(EXIT_HOOKS);

    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
    ];
    conductor::hook(SHUTDOWN_HOOKS);
}

fn spawn_entry() -> Result<Option<alloc::sync::Arc<crate::work::unit::task::Task>>, BootError> {
    let Some(region) = machine::info().initrd() else {
        return Ok(None);
    };
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(region.base as *const u8, region.size) };
    let capsule =
        crate::platform::initrd::entry_image(blob).ok_or(BootError::InvalidEntryImage {
            base: region.base,
            size: region.size,
        })?;
    let team =
        crate::work::unit::capsule::assemble(capsule).map_err(|source| BootError::Mapping {
            operation: MapOperation::AssembleCapsule,
            source,
        })?;

    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view_reservation = team
        .space
        .allocate_low(crate::work::unit::space::RegionId::Normal, view_size)
        .map_err(|source| BootError::Mapping {
            operation: MapOperation::ImageView,
            source,
        })?;
    // SAFETY: the initrd region remains reserved for the boot task's lifetime.
    let view_mapping = unsafe {
        team.space.map_borrowed(
            view_reservation,
            crate::memory::manager::addr::PhysAddr::from_raw(region.base),
            read_only(),
            read_only(),
        )
    }
    .map_err(|source| BootError::Mapping {
        operation: MapOperation::ImageView,
        source,
    })?;
    let view = view_mapping.reservation.start();

    let mut registry = crate::resource::Registry::default();
    crate::platform::devices::register(&mut registry).map_err(|source| BootError::Resources {
        operation: ResourceOperation::Devices,
        source,
    })?;
    crate::runtime::switcher::trap::resources::register(&mut registry).map_err(|source| {
        BootError::Resources {
            operation: ResourceOperation::Traps,
            source,
        }
    })?;
    crate::runtime::switcher::envcall::resources::register(&mut registry).map_err(|source| {
        BootError::Resources {
            operation: ResourceOperation::Calls,
            source,
        }
    })?;
    let resources = registry.freeze().map_err(|e| BootError::Resources {
        operation: ResourceOperation::Freeze,
        source: e.into_parts().0,
    })?;
    let ledger_len =
        crate::resource::boot::size(resources.len()).map_err(|source| BootError::Resources {
            operation: ResourceOperation::LedgerSize,
            source,
        })?;
    let (ledger_pa, ledger_bytes) = crate::resource::boot::block();
    let ledger_reservation = team
        .space
        .allocate_low(crate::work::unit::space::RegionId::Normal, ledger_bytes)
        .map_err(|source| BootError::Mapping {
            operation: MapOperation::LedgerView,
            source,
        })?;
    // SAFETY: the boot ledger is static memory retained for the initial task.
    let ledger_mapping = unsafe {
        team.space.map_borrowed(
            ledger_reservation,
            crate::memory::manager::addr::PhysAddr::from_raw(ledger_pa),
            read_only(),
            read_only(),
        )
    }
    .map_err(|source| BootError::Mapping {
        operation: MapOperation::LedgerView,
        source,
    })?;
    let ledger = ledger_mapping.reservation.start();

    let mut args = [0usize; env::ledger::args::LEN];
    args[env::ledger::args::VIEW] = view.as_usize();
    args[env::ledger::args::VIEW_LEN] = region.size;
    args[env::ledger::args::LEDGER] = ledger.as_usize();
    args[env::ledger::args::LEDGER_LEN] = ledger_len;
    let mut words = alloc::vec::Vec::new();
    words
        .try_reserve_exact(args.len())
        .map_err(|_| BootError::Bootstrap(MapError::OutOfMemory))?;
    words.extend_from_slice(&args);
    let bootstrap = team
        .task()
        .args(words)
        .hold()
        .map_err(BootError::Bootstrap)?;
    let entries = resources
        .grant(&bootstrap)
        .map_err(|e| BootError::Resources {
            operation: ResourceOperation::Grant,
            source: e.into_parts().0,
        })?;
    crate::resource::boot::write(&entries).map_err(|source| BootError::Resources {
        operation: ResourceOperation::WriteLedger,
        source,
    })?;
    crate::work::unit::task::Task::release(&bootstrap).map_err(|source| {
        BootError::BootstrapRelease {
            task: bootstrap.ident.id,
            source,
        }
    })?;

    #[cfg(debug_assertions)]
    team.space.tree.audit();

    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.tree.audit();

    Ok(Some(bootstrap))
}

fn read_only() -> crate::memory::manager::entry::PteFlags {
    use crate::memory::manager::entry::PteFlags;
    PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D
}

fn boot_harts() -> Result<(), BootError> {
    let me = hart::hart_id();
    hart::mark_hart_started(me);
    let count = hart::hart_count();
    let entry = core::ptr::addr_of!(_boot_entry) as usize;
    for hart in 0..count {
        if HartId::new(hart) == me {
            continue;
        }
        let stack_top = boot_stack(hart);
        trace::note(trace::EventKind::Boot(trace::BootEvent::Launch { hart }));
        let r = sbi::HsmCall::new(sbi::fid::Hsm::Start)
            .args(sbi::ecall::SArgs {
                a0: hart,
                a1: entry,
                a2: stack_top,
                ..Default::default()
            })
            .call();
        r.map_err(|source| BootError::HartStart {
            hart: HartId::new(hart),
            source,
        })?;
        hart::mark_hart_started(HartId::new(hart));
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    let me = hart::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .tree
        .translate(hart::hart_frame())
        .expect("kernel frame not mapped")
        .0;
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    unsafe {
        hart::mark_online();
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma", "fence.i");
    }
    arm_hart();
    crate::memory::manager::asid::occupy(crate::memory::manager::asid::Asid::kernel());
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done {
        hart: me.get(),
    }));
    scheduler::boot::idle()
}

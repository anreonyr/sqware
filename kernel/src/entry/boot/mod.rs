pub(crate) mod error;
mod init;
pub(crate) mod initrd;
pub(crate) mod devices;
pub(crate) mod resources;
mod stack;
pub(crate) mod unit;

pub use error::{BootError, MapOperation, ResourceOperation, fail};
pub use init::init;

use ::core::arch::global_asm;

use riscv::register::satp;

use crate::machine::hart::{self, HartId};
use crate::machine::image::{ROOT_STACK_CANARY, root_stack_base};
use crate::core::memory::PAGE_SIZE;
use crate::core::memory::nest::{Access, NestError, RegionId, identity};
use crate::machine::mode;
use crate::machine::board;
use crate::core::work::record::trace;
use crate::machine::switcher::context::TrapContext;
use crate::machine::switcher::trampoline::restore;
use crate::machine::switcher::stack::{arm_hart, boot_stack};
use crate::core::work::room::{conductor, scheduler};
use crate::core::work::unit::team::kernel;

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

pub fn start() -> Result<(), BootError> {
    scheduler::init().map_err(BootError::Scheduler)?;
    register_runtime_hooks();

    // 锁序检测已经在 `init` 里启用（那里在分配器就绪后、被验证调用之前）；
    // 这里不再重复初始化，重复调用会以 AlreadyInit 报错。
    #[cfg(debug_assertions)]
    crate::entry::check::depend::accept();

    #[cfg(debug_assertions)]
    crate::entry::check::run();

    if crate::testing() && board::info().initrd().is_none() {
        return Err(BootError::MissingImage);
    }

    if let Some(entry) = spawn_entry()? {
        crate::core::work::room::halt::beacon::arm(&entry);
    }

    crate::core::work::unit::tally::rooted();

    boot_harts()?;

    #[cfg(debug_assertions)]
    if crate::testing() {
        crate::core::work::record::ipi::run("early", None);
        crate::core::work::record::ipi::start_delayed();
    }

    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );
    Ok(())
}

pub fn run() -> ! {
    restore(conductor::run::run())
}

fn register_runtime_hooks() {
    use crate::core::work::room::halt;
    use crate::core::work::room::messenger;

    static EXIT_HOOKS: &[fn(&alloc::sync::Arc<crate::core::work::unit::task::Task>)] =
        &[crate::core::work::room::messenger::doom, crate::core::work::pend::doom];
    messenger::hook(EXIT_HOOKS);

    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::entry::report::shutdown::report,
        crate::core::work::room::scheduler::rip,
        crate::core::memory::flat::allocator::block::flush,
        crate::entry::report::shutdown::verdict,
    ];
    halt::hook(SHUTDOWN_HOOKS, crate::entry::report::shutdown::waiting);
}

fn spawn_entry() -> Result<Option<alloc::sync::Arc<crate::core::work::unit::task::Task>>, BootError> {
    let Some(region) = board::info().initrd() else {
        return Ok(None);
    };
    let blob: &'static [u8] =
        unsafe { ::core::slice::from_raw_parts(region.base as *const u8, region.size) };
    let capsule =
        crate::entry::boot::initrd::entry_image(blob).ok_or(BootError::InvalidEntryImage {
            base: region.base,
            size: region.size,
        })?;
    let team =
        crate::core::work::unit::capsule::assemble(capsule).map_err(|source| BootError::Mapping {
            operation: MapOperation::AssembleCapsule,
            source,
        })?;

    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view_reservation = team
        .space
        .memory
        .allocate_low(RegionId::Normal, view_size)
        .map_err(|source| BootError::Mapping {
            operation: MapOperation::ImageView,
            source,
        })?;
    let view_mapping = unsafe {
        team.space.memory.map_borrowed(
            view_reservation,
            crate::core::memory::address::PhysAddr::from_raw(region.base),
            Access::READ,
            Access::READ,
        )
    }
    .map_err(|source| BootError::Mapping {
        operation: MapOperation::ImageView,
        source,
    })?;
    let view = view_mapping.address();

    let mut registry = crate::entry::boot::resources::Registry::default();
    crate::entry::boot::devices::register(&mut registry).map_err(|source| BootError::Resources {
        operation: ResourceOperation::Devices,
        source,
    })?;
    crate::entry::trap::resources::register(&mut registry).map_err(|source| {
        BootError::Resources {
            operation: ResourceOperation::Traps,
            source,
        }
    })?;
    crate::entry::call::resources::register(&mut registry).map_err(|source| {
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
        crate::entry::boot::resources::ledger::size(resources.len()).map_err(|source| BootError::Resources {
            operation: ResourceOperation::LedgerSize,
            source,
        })?;
    let (ledger_pa, ledger_bytes) = crate::entry::boot::resources::ledger::block();
    let ledger_reservation = team
        .space
        .memory
        .allocate_low(RegionId::Normal, ledger_bytes)
        .map_err(|source| BootError::Mapping {
            operation: MapOperation::LedgerView,
            source,
        })?;
    let ledger_mapping = unsafe {
        team.space.memory.map_borrowed(
            ledger_reservation,
            crate::core::memory::address::PhysAddr::from_raw(ledger_pa),
            Access::READ,
            Access::READ,
        )
    }
    .map_err(|source| BootError::Mapping {
        operation: MapOperation::LedgerView,
        source,
    })?;
    let ledger = ledger_mapping.address();

    let mut args = [0usize; env::ledger::args::LEN];
    args[env::ledger::args::VIEW] = view.as_usize();
    args[env::ledger::args::VIEW_LEN] = region.size;
    args[env::ledger::args::LEDGER] = ledger.as_usize();
    args[env::ledger::args::LEDGER_LEN] = ledger_len;
    let mut words = alloc::vec::Vec::new();
    words
        .try_reserve_exact(args.len())
        .map_err(|_| BootError::Bootstrap(NestError::OutOfMemory))?;
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
    crate::entry::boot::resources::ledger::write(&entries).map_err(|source| BootError::Resources {
        operation: ResourceOperation::WriteLedger,
        source,
    })?;
    crate::core::work::unit::task::Task::release(&bootstrap).map_err(|source| {
        BootError::BootstrapRelease {
            task: bootstrap.ident.id,
            source,
        }
    })?;

    #[cfg(debug_assertions)]
    team.space.memory.audit();

    #[cfg(debug_assertions)]
    kernel()
        .expect("kernel team not initialized")
        .space
        .memory
        .audit();

    Ok(Some(bootstrap))
}

fn boot_harts() -> Result<(), BootError> {
    let me = hart::hart_id();
    let count = hart::hart_count();
    let entry = ::core::ptr::addr_of!(_boot_entry) as usize;
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
    }
    Ok(())
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    let me = hart::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .memory
        .physical_at(hart::hart_frame())
        .expect("kernel frame not mapped");
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    unsafe {
        hart::mark_online();
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        ::core::arch::asm!("sfence.vma", "fence.i");
    }
    arm_hart();
    identity::occupy(crate::core::memory::nest::Asid::kernel());
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done {
        hart: me.get(),
    }));
    run()
}

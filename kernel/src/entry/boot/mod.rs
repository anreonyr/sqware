pub(crate) mod devices;
pub(crate) mod error;
mod init;
pub(crate) mod initrd;
pub(crate) mod resources;
mod stack;
pub(crate) mod unit;

pub use error::{BootError, MapOperation, ResourceOperation, fail};
pub use init::init;

use ::core::arch::global_asm;

use alloc::sync::Arc;

use riscv::register::satp;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::flat::{BorrowedRegion, Flat, FlatRange};
use crate::core::memory::nest::table::PteFlags;
use crate::core::memory::nest::{
    Access, ActiveRange, Nest, NestError, Placement, RegionId, identity,
};
use crate::core::work::record::trace;
use crate::core::work::room::{conductor, scheduler};
use crate::core::work::unit::team::kernel;
use crate::machine::board;
use crate::machine::hart::{self, HartId};
use crate::machine::image::{ROOT_STACK_CANARY, root_stack_base};
use crate::machine::mode;
use crate::machine::switcher::context::TrapContext;
use crate::machine::switcher::stack::{arm_hart, boot_stack};
use crate::machine::switcher::trampoline::restore;

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

    static EXIT_HOOKS: &[fn(&alloc::sync::Arc<crate::core::work::unit::task::Task>)] = &[
        crate::core::work::room::messenger::doom,
        crate::core::work::pend::doom,
    ];
    messenger::hook(EXIT_HOOKS);

    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::entry::report::shutdown::report,
        crate::core::work::room::scheduler::rip,
        crate::core::memory::flat::allocator::block::flush,
        crate::entry::report::shutdown::verdict,
    ];
    halt::hook(SHUTDOWN_HOOKS, crate::entry::report::shutdown::waiting);
}

fn spawn_entry() -> Result<Option<alloc::sync::Arc<crate::core::work::unit::task::Task>>, BootError>
{
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
    let team = crate::core::work::unit::capsule::assemble(capsule).map_err(|source| {
        BootError::Mapping {
            operation: MapOperation::AssembleCapsule,
            source,
        }
    })?;

    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view = map_borrowed(&team.space.nest, PhysAddr::from_raw(region.base), view_size).map_err(
        |source| BootError::Mapping {
            operation: MapOperation::ImageView,
            source,
        },
    )?;

    let mut registry = crate::entry::boot::resources::Registry::default();
    crate::entry::boot::devices::register(&mut registry).map_err(|source| {
        BootError::Resources {
            operation: ResourceOperation::Devices,
            source,
        }
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
    let ledger_len = crate::entry::boot::resources::ledger::size(
        resources.len() + crate::entry::boot::resources::RETENTION_ENTRIES,
    )
    .map_err(|source| BootError::Resources {
        operation: ResourceOperation::LedgerSize,
        source,
    })?;
    let (ledger_pa, ledger_bytes) = crate::entry::boot::resources::ledger::block();
    let ledger = map_borrowed(
        &team.space.nest,
        PhysAddr::from_raw(ledger_pa),
        ledger_bytes,
    )
    .map_err(|source| BootError::Mapping {
        operation: MapOperation::LedgerView,
        source,
    })?;

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
    // 可信控制者那一枚独立保留授权：只在这里铸、只发给这一个任务，
    // 与上面那批具名资源同走账本一条路。
    let mut entries = entries;
    entries
        .try_reserve(crate::entry::boot::resources::RETENTION_ENTRIES)
        .map_err(|_| BootError::Bootstrap(NestError::OutOfMemory))?;
    entries.push(
        crate::entry::boot::resources::grant_retention(&bootstrap).map_err(|source| {
            BootError::Resources {
                operation: ResourceOperation::Grant,
                source,
            }
        })?,
    );
    crate::entry::boot::resources::ledger::write(&entries).map_err(|source| {
        BootError::Resources {
            operation: ResourceOperation::WriteLedger,
            source,
        }
    })?;
    crate::core::work::unit::task::Task::release(&bootstrap).map_err(|source| {
        BootError::BootstrapRelease {
            task: bootstrap.ident.id,
            source,
        }
    })?;

    #[cfg(debug_assertions)]
    team.space.nest.audit();

    #[cfg(debug_assertions)]
    kernel()
        .expect("kernel team not initialized")
        .space
        .nest
        .audit();

    Ok(Some(bootstrap))
}

/// 把一段调用方仍保留的物理区间挂进 `Normal` 窗口的最低空档（只读）。
///
/// 借用凭证与来源在登记占用之前就已备齐：来源失败不留下无人持有的地址。
fn map_borrowed(nest: &Arc<Nest>, physical: PhysAddr, size: usize) -> Result<VirtAddr, NestError> {
    let borrowed = unsafe { BorrowedRegion::new(physical, size)? };
    let source = Flat::from_region(borrowed)?.whole();
    let reservation = nest.reserve(Placement::lowest(RegionId::Normal, size, PAGE_SIZE)?)?;
    let view = nest.attach(reservation, source, Access::READ, Access::READ)?;
    Ok(view.initial_start())
}

/// 启动静态挂载：按 `Exact` 在 `start` 登记真实占用，再把已备好的来源挂上去。
///
/// 活动形状在登记占用之前备齐；`Static` 区只有重叠检查、没有动态窗口，
/// 所以这里不走窗口校验。
fn attach_exact(
    nest: &Arc<Nest>,
    start: VirtAddr,
    source: FlatRange,
    access: Access,
    attrs: PteFlags,
) -> Result<(), NestError> {
    let size = source
        .page_count()
        .checked_bytes()
        .ok_or(NestError::NotAligned)?;
    let active = ActiveRange::new(start, size, access, access)?;
    let reservation = nest.reserve(Placement::exact(RegionId::Static, start, size)?)?;
    nest.attach_shaped(reservation, source, attrs, &[active])?;
    Ok(())
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
        .nest
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

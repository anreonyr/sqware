use core::arch::global_asm;

use alloc::format;
use alloc::vec;
use riscv::register::satp;

use crate::console::Sink;
use crate::hart::{self, HartId};
use crate::layout::{HART_FRAME_BASE, TRAP_STACK_SLOT_SIZE};
use crate::layout::{ROOT_STACK_CANARY, root_stack_base};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::mode;
use crate::platform::machine;
use crate::runtime::diagnose::report::Report;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, restore};
use crate::runtime::switcher::trap::{arm_hart, trap_stack, trap_stack_base, trap_stack_edge};
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

pub fn banner() {
    let m = machine::info();
    let mut r = Report::default();
    {
        let p = r.paragraph("banner", None);
        for (label, value) in [
            ("hart count", format!("{} H", m.hart.count)),
            ("hart this", format!("{}", hart::hart_id())),
            ("timebase", format!("{} Hz", m.hart.hertz)),
            (
                "dram",
                format!("{:#x}..{:#x}", m.dram.base, m.dram.range().end),
            ),
            (
                "free",
                format!("{:#x}..{:#x}", m.free.base, m.free.range().end),
            ),
            ("trap vector", format!("{:#x}", alltraps_va())),
            (
                "kernel frames",
                format!(
                    "{:#x}..{:#x}",
                    HART_FRAME_BASE.as_usize(),
                    HART_FRAME_BASE.as_usize() + m.hart.count * PAGE_SIZE
                ),
            ),
            (
                "trap stack",
                format!(
                    "{:#x}..{:#x}",
                    trap_stack_base(HartId::new(0)).as_usize(),
                    trap_stack_edge(HartId::new(0)).as_usize()
                ),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    hart::hart_id(),
                    trap_stack_base(hart::hart_id()).as_usize(),
                    trap_stack_edge(hart::hart_id()).as_usize()
                ),
            ),
        ] {
            p.items.push(vec![Some(label.into()), Some(value)]);
        }
    }
    let sealed = r.seal();
    let mut sink = Sink;
    crate::runtime::diagnose::render::render(sealed, &mut sink, 0);
}

pub fn init() {
    scheduler::boot::init();
    register_runtime_hooks();

    #[cfg(debug_assertions)]
    crate::lock::init_depend(hart::hart_count()).expect("depend init failed");

    // **（这一格是 release 档编不过的当场修复）**：健康面那九例整体 gate 进了
    // `debug_assertions`（`kernel/src/health/mod.rs` 头上那一句），而这一句**没跟着 gate**
    // ——于是 `cargo build -p kernel --release`（以及 `cargo qtest --scene` 那条 release 路）
    // 当场 E0433：`cannot find health in the crate root`。两侧同一个闸：这里补上。
    #[cfg(debug_assertions)]
    crate::health::run();

    if crate::testing() && machine::info().initrd().is_none() {
        panic!(
            "整机用例没有镜像：`cargo-qtest` 不带 `-initrd`。请用 \
             `nu scripts/qtest.nu --scene <景>` 跑（本检查只在测试模式生效）"
        );
    }

    if let Some(entry) = spawn_entry().expect("boot spawn failed") {
        crate::work::room::scheduler::core::beacon_arm(&entry);
    }

    crate::work::room::conductor::rooted();

    boot_harts();

    #[cfg(debug_assertions)]
    {
        crate::runtime::diagnose::ipi::run("early", None);
        crate::runtime::diagnose::ipi::start_delayed();
    }

    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );
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

    crate::work::unit::gate::install(crate::work::room::scheduler::core::roster);

    const SHUTDOWN_HOOKS: &[fn()] = &[
        crate::work::room::scheduler::core::rip,
        crate::memory::allocator::block::flush,
    ];
    conductor::hook(SHUTDOWN_HOOKS);
}

fn spawn_entry() -> Result<Option<alloc::sync::Arc<crate::work::unit::task::Task>>, MapError> {
    let Some(region) = machine::info().initrd() else {
        return Ok(None);
    };
    let blob: &'static [u8] =
        unsafe { core::slice::from_raw_parts(region.base as *const u8, region.size) };
    let capsule = crate::platform::initrd::entry_image(blob).expect("initrd: capsule missing");
    let team = crate::work::unit::capsule::assemble(capsule).expect("assemble boot capsule");

    let view_size = region.size.next_multiple_of(PAGE_SIZE);
    let view = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = crate::work::unit::space::window::HeapWindow::locate(inner, view_size)?;
            inner.allocate(crate::work::unit::space::SegmentKind::Normal, va.as_usize(), view_size)?;
            inner.borrow(
                va,
                crate::memory::manager::addr::PhysAddr::from_raw(region.base),
                view_size,
                read_only(),
            )?;
            Ok(va)
        },
    )?;

    let devices = crate::platform::devices::scan();
    let (pairs_pa, pairs_bytes) = crate::platform::devices::block();
    let pairs = team.space.with_flush(
        |inner| -> Result<crate::memory::manager::addr::VirtAddr, MapError> {
            let va = crate::work::unit::space::window::HeapWindow::locate(inner, pairs_bytes)?;
            inner.allocate(crate::work::unit::space::SegmentKind::Normal, va.as_usize(), pairs_bytes)?;
            inner.borrow(
                va,
                crate::memory::manager::addr::PhysAddr::from_raw(pairs_pa),
                pairs_bytes,
                read_only(),
            )?;
            Ok(va)
        },
    )?;

    let mut args = [0usize; env::ledger::args::LEN];
    args[env::ledger::args::VIEW] = view.as_usize();
    args[env::ledger::args::VIEW_LEN] = region.size;
    args[env::ledger::args::PAIRS] = pairs.as_usize();
    args[env::ledger::args::COUNT] = devices.len();
    let bootstrap = team.task().args(args.to_vec()).hold()?;
    crate::work::unit::task::Task::release(&bootstrap).expect("freshly held task must release");
    crate::platform::devices::install(&bootstrap, devices);

    #[cfg(debug_assertions)]
    team.space.audit();

    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.audit();

    Ok(Some(bootstrap))
}

fn read_only() -> crate::memory::manager::entry::PteFlags {
    use crate::memory::manager::entry::PteFlags;
    PteFlags::V | PteFlags::R | PteFlags::A | PteFlags::D
}

fn boot_harts() {
    let me = hart::hart_id();
    hart::mark_hart_started(me);
    let count = hart::hart_count();
    let entry = core::ptr::addr_of!(_boot_entry) as usize;
    for hart in 0..count {
        if HartId::new(hart) == me {
            continue;
        }
        let stack_top = trap_stack() + (hart + 1) * TRAP_STACK_SLOT_SIZE;
        trace::note(trace::EventKind::Boot(trace::BootEvent::Launch { hart }));
        let r = sbi::HsmCall::new(sbi::fid::Hsm::Start)
            .args(sbi::ecall::SArgs {
                a0: hart,
                a1: entry,
                a2: stack_top,
                ..Default::default()
            })
            .call();
        if r.is_err() {
            panic!("failed to start hart {hart}: {r:?}");
        }
        hart::mark_hart_started(HartId::new(hart));
    }
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    let me = hart::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .translate(hart::hart_frame())
        .expect("kernel frame not mapped")
        .0;
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    unsafe {
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

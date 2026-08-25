// 启动（boot）— 把 work 子系统拉起到首个用户任务。

use core::arch::global_asm;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec;
use riscv::register::{satp, sie, stvec};

use crate::console::Sink;
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::mode;
use crate::runtime::diagnose::report::Report;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{
    alltraps_va, restore, trap_stack_bottom, trap_stack_top,
};
use crate::work::room::scheduler;
use crate::work::unit::space::{KERNEL_FRAME_BASE, SpaceBuilder, kernel_frame_pa};
use crate::work::unit::team::kernel;
use crate::work::unit::{loader, team};
use crate::{machine, putln};

global_asm!(
    ".section .text.boot",
    ".align 2",
    ".globl _boot_entry",
    "_boot_entry:",
    "    mv   tp, a0",     // hartid → tp（与 _start 一致；hart_id() 读 tp）
    "    csrc sstatus, 2", // 清 SIE：内核态恒关中断（同 _start）
    "    mv   sp, a1",     // opaque = 本 hart trap 栈顶（HSM Start 传入；寄存器传递）
    "    call boot_main",
);

unsafe extern "C" {
    /// 副核入口（HSM Start 的 start_addr；内核镜像恒等加载，链接地址即物理地址）。
    static _boot_entry: u8;
}

/// SBI 式启动横幅：机器/板级 + 陷阱布局两块投稿成一个 banner 段落；值用
/// format! 拼装。
pub fn banner() {
    let m = machine::info();
    let mut r = Report::default();
    {
        let p = r.paragraph("banner", None);
        for (label, value) in [
            ("hart count", format!("{} H", m.hart)),
            ("hart this", format!("{}", machine::hart_id())),
            ("timebase", format!("{} Hz", m.hertz)),
            (
                "dram",
                format!("{:#x}..{:#x}", m.dram.base, m.dram.range().end),
            ),
            (
                "free",
                format!("{:#x}..{:#x}", m.free.base, m.free.range().end),
            ),
            ("uart", format!("{:#x}", m.uart.base)),
            ("plic", format!("{:#x}", m.plic.base)),
            ("clint", format!("{:#x}", m.clint.base)),
            ("trap vector", format!("{:#x}", alltraps_va())),
            (
                "kernel frames",
                format!(
                    "{:#x}..{:#x}",
                    KERNEL_FRAME_BASE.as_usize(),
                    KERNEL_FRAME_BASE.as_usize() + m.hart * PAGE_SIZE
                ),
            ),
            (
                "trap stack",
                format!("{:#x}..{:#x}", trap_stack_bottom(0), trap_stack_top(0)),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    machine::hart_id(),
                    trap_stack_bottom(machine::hart_id()),
                    trap_stack_top(machine::hart_id())
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

/// 启动多任务：spawn 演示团队后进入首个线程。
pub fn init() -> ! {
    // per-hart 调度器状态按实际核数（DTB）动态分配——先于任何调度器访问
    scheduler::init();

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(machine::hart_count()).expect("depend init failed");

    // 健康检查（spare 预算验收恒跑 + PT 回收自测 debug）：任一失败
    // fail-fast（panic → crash scene）。
    crate::health::run();

    // 记录内核持久帧基线：**一切任务 spawn 之前**。此后在途帧只增任务所有；
    // 关机时全部归还（零泄漏审计）。区间窗口元数据是随任务存活的瞬态
    // （任务栈/帧条目随 reclaim 摘除、团队 drop 归还）——基线后创建、关机前
    // 全部释放，block 池净零回落；基线前只留静态内核结构（镜像、per-hart 帧、
    // 内核空间 durable 映射），关机不释放，账平。
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::record_baseline();

    // 演示程序均为内嵌 ELF，经装载生成任务并入队；错误一律 `?` 上抛至本边界。
    spawn_demos().expect("boot spawn failed");

    // 完整性审计（debug）：boot 收尾全量核对。
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::audit();

    // 多核：HSM 拉起其余副核。
    boot_harts();

    // 主内核栈（boot 栈）将永久离开前校验 canary：boot 期栈溢出即使未越过
    // guard 页（4 KiB 内）也会在此暴露，且不必等缺页死机。
    let boot_guard = unsafe { (crate::kernel_stack_base() as *const usize).read() };
    assert!(
        boot_guard == crate::KERNEL_STACK_CANARY,
        "main kernel stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走，见 scheduler::enter_first_task）
    restore(scheduler::run())
}

/// 生成全部演示任务（用户 + 内核 ktask）；错误统一 `?` 上抛。返回前所有任务已入队。
fn spawn_demos() -> Result<(), MapError> {
    // Team1「threader」：双线程共享同一地址空间——线程参数 a0 分支 'A'/'B'。先 Team 后 Task。
    // let (team1, entry1) = load_user(
    //     &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-threader")[..],
    // );
    // team1.task().name("thread-A").entry(entry1).arg(0).spawn()?;
    // team1.task().name("thread-B").entry(entry1).arg(1).spawn()?;
    // drop(team1); // 构造期句柄用完即弃——团队由它的线程持有

    // 单线程团队回归
    for (elf, name) in [
        (&include_bytes!(env!("USER_MMAPER"))[..], "mmaper"),
        (&include_bytes!(env!("USER_HEAPER"))[..], "heaper"),
        (&include_bytes!(env!("USER_SPAWNER"))[..], "spawner"),
    ] {
        let (team, entry) = load_user(elf);
        team.task().name(name).entry(entry).spawn()?;
        // debug: 每个演示空间 簿记↔页表 一致性审计
        #[cfg(debug_assertions)]
        team.space.audit();
    }

    // 内核任务（ktask）：挂 kernel 团队单例。
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("ktask")
        .closure(|| {
            putln!("ktask");
            // panic!("Shit");
        })?;
    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.audit();
    Ok(())
}

/// 内嵌用户 ELF 经解析装载生成 Team；返回 (Team, 绝对入口)。
fn load_user(elf: &'static [u8]) -> (Arc<team::Team>, VirtAddr) {
    let parsed = crate::work::unit::parser::parse(elf).expect("parse user elf");
    let space = SpaceBuilder::user().build().expect("space failed");
    let loaded = loader::load(space, elf, &parsed).expect("load user elf");
    let entry = loaded.entry;
    // 符号表：内嵌 ELF 的 .symtab/.strtab → ElfTable（失败则 None，只影响符号化不碍装载）
    let elftable = crate::work::unit::parser::tables(elf)
        .ok()
        .and_then(|(s, ss)| crate::work::unit::elftable::ElfTable::from_sections(s, ss))
        .map(Arc::new);
    let team = team::TeamBuilder::new(loaded.space)
        .elftable(elftable)
        .spawn();
    (team, entry)
}

/// boot 启动：HSM `hart_start` 逐个拉起 hart 1..count-1。
///
/// start_addr = _boot_entry（恒等映射地址）；opaque = 该 hart 的 trap 栈顶
/// ——副核入口直接 `mv sp, a1`，寄存器传递，无需共享内存同步。
fn boot_harts() {
    // boot hart 不一定是 0（QEMU/OpenSBI 随机选）——它已在运行，须标记为已启动，
    // 并只 HSM 拉起**其它** hart（0..count 中除自身外全部）。
    let me = machine::hart_id();
    machine::mark_hart_started(me);
    let count = machine::hart_count();
    let entry = core::ptr::addr_of!(_boot_entry) as usize;
    for hart in 0..count {
        if hart == me {
            continue;
        }
        let stack_top = crate::runtime::switcher::trampoline::trap_stack_top(hart);
        // putln!("hart {me}: starting hart {hart} @ {entry:#x}, trap stack {stack_top:#x}");
        // 同事件也进 trace（hart 0 窗口）：崩溃回放可见启动序列。
        trace::note(trace::EventKind::Boot(trace::BootEvent::Launch { hart }));
        let r = sbi::HsmCall::new(sbi::fid::Hsm::Start)
            .args(sbi::scall::SArgs {
                a0: hart,
                a1: entry,
                a2: stack_top,
                ..Default::default()
            })
            .call();
        if r.is_err() {
            panic!("failed to start hart {hart}: {r:?}");
        }
        machine::mark_hart_started(hart);
    }
}

/// 副核主流程：per-hart CSR 配置后进入 idle。
#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    // 副核 per-hart 初始化：satp = 共享内核 token（从内核帧读）、stvec、sscratch、sie。
    let ktc = kernel_frame_pa(0);
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    // 探测所得模式 token：低 44 位 ppn、[63:44] asid/模式（字段访问器拆解，
    // 无裸位运算；模式位随 mode()，副核与主核同模式）
    unsafe {
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma");
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
        sie::set_ssoft(); // SSIP 使能：WFI 休眠唤醒
    }
    // 启动完成写进 trace（直打控制台会扰 panic 现场）。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done {
        hart: machine::hart_id(),
    }));
    scheduler::idle()
}

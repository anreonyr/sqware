// 启动（boot）— 把 work 子系统拉起到首个用户任务。

use core::arch::global_asm;

use alloc::format;
use alloc::sync::Arc;
use alloc::vec;
use riscv::register::satp;

use crate::console::Sink;
use crate::layout::{HART_FRAME_BASE, TRAP_STACK_SLOT_SIZE};
use crate::machine::{ROOT_STACK_CANARY, root_stack_base};
use crate::memory::PAGE_SIZE;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::mode;
use crate::runtime::diagnose::report::Report;
use crate::runtime::diagnose::trace;
use crate::runtime::switcher::context::TrapContext;
use crate::runtime::switcher::trampoline::{alltraps_va, restore};
use crate::runtime::switcher::trap::{arm_hart, trap_stack, trap_stack_base, trap_stack_edge};
use crate::work::room::conductor;
use crate::work::unit::space::SpaceBuilder;
use crate::work::unit::team::kernel;
use crate::work::unit::{loader, team};
use crate::{machine, putln};

global_asm!(
    ".section .text.boot",
    ".align 2",
    ".globl _boot_entry",
    "_boot_entry:",
    "    la   t0, PER_HART", // &PER_HART[0]（恒等映射，Bare 下 PC 相对即物理地址）
    "    slli t1, a0, 5",    // a0 = hartid（HSM Start 传入）· 32（PerHart 槽宽 2⁵）
    "    add  tp, t0, t1",   // tp = 本 hart PerHart 指针（入口约定，见 `hart_id()`）
    "    csrc sstatus, 2",
    "    mv   sp, a1", // opaque = 本 hart trap 栈顶（HSM Start 传入）
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
                    HART_FRAME_BASE.as_usize(),
                    HART_FRAME_BASE.as_usize() + m.hart * PAGE_SIZE
                ),
            ),
            (
                "trap stack",
                format!(
                    "{:#x}..{:#x}",
                    trap_stack_base(0).as_usize(),
                    trap_stack_edge(0).as_usize()
                ),
            ),
            (
                "trap stack this",
                format!(
                    "{} @ {:#x}..{:#x}",
                    machine::hart_id(),
                    trap_stack_base(machine::hart_id()).as_usize(),
                    trap_stack_edge(machine::hart_id()).as_usize()
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
    conductor::boot::init();

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(machine::hart_count()).expect("depend init failed");

    // 健康检查（spare 预算验收恒跑 + PT 回收自测 debug）：任一失败
    // fail-fast（panic → crash scene）。
    crate::health::run();

    // 记录内核持久帧基线（一切任务 spawn 之前；关机时审计零泄漏）
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::record_baseline();

    spawn_demos().expect("boot spawn failed");

    // 完整性审计（debug）
    #[cfg(debug_assertions)]
    crate::memory::allocator::fence::audit::audit();

    // 多核：HSM 拉起其余副核。
    boot_harts();

    // ROOT 栈完整性审核：boot 期栈溢出即使未越过 guard 页（4 KiB 内）也会在此暴露。
    let boot_guard = unsafe { (root_stack_base() as *const usize).read() };
    assert!(
        boot_guard == ROOT_STACK_CANARY,
        "ROOT stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走）
    restore(conductor::trap::run())
}

/// 生成全部演示任务（用户 + 内核 ktask）；错误统一 `?` 上抛。
fn spawn_demos() -> Result<(), MapError> {
    // 单线程团队回归
    for (elf, name) in [
        (&include_bytes!(env!("USER_HEAPER"))[..], "heaper"),
        (&include_bytes!(env!("USER_SPAWNER"))[..], "spawner"),
        (&include_bytes!(env!("USER_MAILER"))[..], "mailer"),
        (&include_bytes!(env!("USER_YIELDER"))[..], "yielder"),
        (&include_bytes!(env!("USER_SLEEPER"))[..], "sleeper"),
        (&include_bytes!(env!("USER_EXITER"))[..], "exiter"),
        (&include_bytes!(env!("USER_STRESSOR"))[..], "stressor"),
        (&include_bytes!(env!("USER_MMAPER"))[..], "mmaper"),
        (&include_bytes!(env!("USER_TLSER"))[..], "tlser"),
        (&include_bytes!(env!("USER_DOCKER"))[..], "docker"),
        (&include_bytes!(env!("USER_LISP"))[..], "lisp"),
    ] {
        let (team, entry) = load_user(elf);
        let mut task = team.task().name(name).entry(entry);
        // lisp 解释器递归求值栈深（其余 demo 16K 缺省即可）
        if name == "lisp" {
            task = task.stack(256 * 1024);
        }
        task.spawn()?;
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
        })?;
    // 可抢占内核验证：常驻循环任务（每轮百万次增量 + 打印进度）。被 S-timer 抢占
    // 恢复正确 → round 顺序不重不乱、n 单调递增、hart 稳定；现场丢失则崩/乱序。
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("preempt")
        .closure(|| {
            let mut n: usize = 0;
            for round in 0..10u32 {
                let start = n;
                for _ in 0..100_000 {
                    n = n.wrapping_add(1);
                }
                crate::putln!(
                    "preempt: round {round} n={n:#x} delta={:#x} hart={}",
                    n.wrapping_sub(start),
                    crate::machine::hart_id()
                );
            }
            crate::putln!("preempt: done");
        })?;
    // 内核任务自愿睡眠（conductor::ktask::park 软陷阱）：睡 200 ms → 唤醒后
    // 打印 → 再睡 → 三轮回后退出。验证存帧/持久化/唤醒恢复/重武装整链。
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("sleep")
        .closure(|| {
            for round in 0..3u32 {
                crate::putln!(
                    "ktask sleep: round {round} @ hart {}",
                    crate::machine::hart_id()
                );
                crate::work::room::conductor::ktask::park(core::time::Duration::from_millis(200));
            }
            crate::putln!("ktask sleep: done");
        })?;
    // 风暴回归：内核任务连环 spawn N 个 closure 子任务（子任务立即退出）。
    // 根因：`Box::try_new_in([u8;4096])`/`PageTable::default()` 按值物化 ~16 KiB
    // 栈帧击穿 16 KiB 任务栈（栈走穿 guard → 未映射区/邻居槽 → 多形态崩溃）。
    // 修复：Box::try_new_zeroed_in（allocate_zeroed，栈上不物化）替换按值 4 KiB 物化。
    storm_ktask(64)?;
    #[cfg(debug_assertions)]
    kernel().expect("kernel team not initialized").space.audit();
    Ok(())
}

/// 风暴 = 内核任务连环 spawn closure 子任务（子任务空跑即退）。
fn storm_ktask(n: usize) -> Result<(), MapError> {
    kernel()
        .expect("kernel team not initialized")
        .task()
        .name("storm")
        .closure(move || {
            let kt = kernel().expect("kernel team not initialized");
            crate::putln!("storm: begin spawn {n}");
            for _i in 0..n {
                kt.task()
                    .name("child")
                    .closure(|| {})
                    .expect("storm spawn child");
            }
            crate::putln!("storm: all {n} spawned");
        })?;
    Ok(())
}

/// 内嵌用户 ELF 经解析装载生成 Team；返回 (Team, 绝对入口)。
fn load_user(elf: &'static [u8]) -> (Arc<team::Team>, VirtAddr) {
    let parsed = crate::work::unit::parser::parse(elf).expect("parse user elf");
    let space = SpaceBuilder::user().build().expect("space failed");
    let loaded = loader::load(space, elf, &parsed).expect("load user elf");
    let entry = loaded.entry;
    // 符号表（失败则 None，只影响符号化不碍装载）
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
        // opaque = 该 hart trap 栈物理栈顶（装配产物块基址 + 布局常量段偏移组装）
        let stack_top = trap_stack() + (hart + 1) * TRAP_STACK_SLOT_SIZE;
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
    // 副核 per-hart 初始化：先取共享内核 token（从**本 hart** 帧读——所有先上
    // 台的核都在 trap::init 填过相同的 kernel_satp，读自身帧语义最贴 per-hart；
    // 其余 per-hart CSR（stvec/sscratch/sie）与 hart 0 走同一原语 trap::arm_hart。
    let me = machine::hart_id();
    let ktc = kernel()
        .expect("kernel team not initialized")
        .space
        .translate(machine::hart_frame())
        .expect("kernel frame not mapped")
        .0;
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    // 探测所得模式 token：低 44 位 ppn、[63:44] asid/模式（字段访问器拆解，
    // 无裸位运算；模式位随 mode()，副核与主核同模式）
    unsafe {
        satp::set(mode::mode(), ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma");
    }
    arm_hart();
    // 启动完成写进 trace（直打控制台会扰 panic 现场）。
    trace::note(trace::EventKind::Boot(trace::BootEvent::Done { hart: me }));
    conductor::boot::idle()
}

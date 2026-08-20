// 启动（boot）— 把 work 子系统拉起到首个用户任务（永不返回）。
//
// 一条纵线：调度器 init（per-hart 状态按 DTB 实际核数分配，先于任何调度器
// 访问）→ debug PT 回收自测 → 构建演示团队/任务（先 Team 后 Task）
// → HSM 拉起副核 → 本核 restore(run()) 进入首任务。
//
// 多核启动（hart B1）：hart 0 完整初始化（trap 栈/canary/spawn）后经 SBI HSM
// `hart_start` 拉起 hart 1..N-1；副核入口（_boot_entry）把 HSM 传入的
// a0=hartid / a1=opaque（= 本 hart trap 栈顶，寄存器传递免共享内存同步）装成
// tp/sp 后进入 boot_main → per-hart CSR 配置 → idle（spin+steal）。

use core::arch::global_asm;
use core::time::Duration;

use alloc::sync::Arc;
use log::info;
use riscv::register::{satp, sie, stvec};

use crate::machine;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr;
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::{alltraps_va, restore};
use crate::work::room::scheduler;
#[cfg(debug_assertions)]
use crate::work::unit;
use crate::work::unit::space::{SpaceBuilder, kernel_frame_pa};
use crate::work::unit::team::kernel;
use crate::work::unit::{loader, team};

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

/// 启动多任务：spawn 演示团队后进入首个线程，永不返回。
///
/// Team1 = 双线程共享空间：首线程 a0=0 计数循环写 'A'（靠抢占轮转）；次线程
/// a0=1 写 'B' 后退出——验证「线程退出、团队/兄弟线程存活」的引用计数语义。
/// 之后是单线程团队回归：A counter（不自让出，靠抢占）→ B yielder（主动让出）
/// → C exiter（写 'C' 后退出）。S-timer 由 runtime::init 武装、trap_handler 内循环重武装。
/// 锁地址符号化回调：内核地址 → (函数名, 偏移)。team=None 只走内核表。
fn kernel_symbolizer(addr: usize) -> Option<(&'static str, usize)> {
    crate::work::unit::elftable::resolve(
        crate::memory::manager::addr::VirtAddr::from_raw(addr),
        None,
    )
}

pub fn init() -> ! {
    // per-hart 调度器状态按实际核数（DTB）动态分配——先于任何调度器访问
    scheduler::init();

    // 值班看护：设阈值并启用（clock 已就绪）。失速 200ms / 锁相持 500ms。
    crate::runtime::watch::threshold(crate::runtime::watch::Threshold {
        hold_timeout: Duration::from_millis(500),
        liveness_timeout: Duration::from_millis(200),
        enabled: true,
    });

    // lockdep 装配（debug 构建）：per-hart 持有集。release 为 no-op。
    // 置于调度器就绪后、spawn 演示任务/HSM 拉起副核前——正是多核 ABBA 的生效窗口。
    #[cfg(debug_assertions)]
    crate::lock::init_depend(machine::hart_count()).expect("depend init failed");
    // 锁地址符号化：depend 打印现场用（未注入则裸地址）。
    crate::lock::set_symbolizer(&kernel_symbolizer);

    // PT 回收自测（debug）：unmap 时中间表必须当场归还——不泄漏、不 double-free
    #[cfg(debug_assertions)]
    unit::pagetable_reclaim();

    // 记录内核持久帧基线：spawn 用户任务前的在途帧 + 内核堆支撑页。此后在途帧
    // 只应增用户任务所有 + 堆支撑页；关机时全部归还，由 tie::halt 的
    // check_baseline 断言零泄漏（见 frame.rs record_baseline/check_baseline）。
    #[cfg(debug_assertions)]
    frame::record_baseline();

    // 全部演示程序均为经 parser → loader → TaskBuilder 装载的**真 ELF**（user crate，
    // 静态链接于 USER_TEXT_BASE 0x10000）。
    //
    // Team1「threader」：双线程共享同一地址空间——线程参数 a0 分支（0 → 'A' 循环、
    // 非 0 → 'B' 循环），多核下分布在两 hart 真实并行。先 Team 后 Task。
    // 全部演示任务（用户 + 内核）经统一 `Team::task()` 入口生成，错误一律 `?` 上抛至本边界。
    spawn_demos().expect("boot spawn failed");

    // 多核：HSM 启动副核（trap 栈/canary 已由 trap::init 就绪；副核 idle 后
    // 经 steal 从队列取活——任务即向各核迁移）
    crate::runtime::watch::suspend();
    boot_harts();
    crate::runtime::watch::resume();

    // 主内核栈（boot 栈）将永久离开前校验 canary：boot 期栈溢出即使未越过
    // guard 页（4 KiB 内）也会在此暴露，且不必等缺页死机。
    let boot_guard = unsafe { (crate::kernel_stack_base() as *const usize).read() };
    assert!(
        boot_guard == crate::KERNEL_STACK_CANARY,
        "main kernel stack overflow during boot: canary corrupted {boot_guard:#x}",
    );

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走，见 scheduler::enter_first_task）
    putln!("task: entering first task");
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

    // 单线程团队回归：counter/yielder/sleeper/exiter 行为不变
    for (elf, name) in [
        // (
        //     &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-counter")[..],
        //     "counter",
        // ),
        // (
        //     &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-yielder")[..],
        //     "yielder",
        // ),
        // (
        //     &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-sleeper")[..],
        //     "sleeper",
        // ),
        (
            &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-exiter")[..],
            "exiter",
        ),
        (
            &include_bytes!("../../target/riscv64gc-unknown-none-elf/debug/user-heaper")[..],
            "heaper",
        ),
        // (
        //     &include_bytes!("../..//target/riscv64gc-unknown-none-elf/debug/user-spawner")[..],
        //     "spawner",
        // ),
    ] {
        let (team, entry) = load_user(elf);
        team.task().name(name).entry(entry).spawn()?;
    }

    // 内核任务（ktask）：挂 kernel 团队单例，经统一 `Team::task().closure`——团队身份
    // 自动定 S 态（SPP=1）、闭包装箱到内核堆、入口为内核 trampoline `ktask_entry`。
    kernel().task().name("ktask").closure(|| {
        putln!("kernel task running");
    })?;
    Ok(())
}

/// 内嵌用户 ELF → parser → loader → Team；返回 (Team, 绝对入口)。
fn load_user(elf: &'static [u8]) -> (Arc<team::Team>, VirtAddr) {
    let parsed = crate::work::unit::parser::parse(elf).expect("parse user elf");
    let space = SpaceBuilder::user().build().expect("space failed");
    let loaded = loader::load(space, elf, &parsed).expect("load user elf");
    let entry = loaded.entry;
    // 符号表：内嵌 ELF 的 .symtab/.strtab → ElfTable（失败则 None，只影响符号化不碍装载）
    let elftable = crate::work::unit::parser::symtabs(elf)
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
        let stack_top = crate::runtime::trampoline::trap_stack_top(hart);
        putln!("hart {me}: starting hart {hart} @ {entry:#x}, trap stack {stack_top:#x}");
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

/// 副核主流程（asm 入口调用）：per-hart CSR 配置后进入 idle（spin+steal）。
#[unsafe(no_mangle)]
pub(crate) extern "C" fn boot_main() -> ! {
    // 副核 per-hart 初始化（HSM 启动后由 boot_main 调用）：
    // satp = 共享内核 token（从内核帧读）、stvec、sscratch、sie。
    // （trap 栈 / canary / 内核帧由 hart 0 在 init 完成；B1 共享内核帧。）
    let ktc = kernel_frame_pa(0);
    let frame = unsafe { &*(ktc.as_usize() as *const TrapContext) };
    let ksatp = frame.kernel_satp;
    // Sv39 token：低 44 位 ppn、[63:44] asid/模式（字段访问器拆解，无裸位运算）
    unsafe {
        satp::set(satp::Mode::Sv39, ksatp.asid(), ksatp.ppn());
        core::arch::asm!("sfence.vma");
        stvec::write(stvec::Stvec::new(alltraps_va(), stvec::TrapMode::Direct));
        core::arch::asm!("csrw sscratch, zero");
        sie::set_stimer();
        sie::set_ssoft(); // SSIP 使能：WFI 休眠唤醒
    }
    info!("runtime \n\t hart {} trap init done", machine::hart_id());
    scheduler::idle()
}
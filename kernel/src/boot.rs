// 启动（boot）— 把 work 子系统拉起到首个用户任务（永不返回）。
//
// 一条纵线：调度器 init（per-hart 状态按 DTB 实际核数分配，先于任何调度器
// 访问）→ debug PT 回收自测 → 构建演示团队/任务（先 Team 后 Task，见 work/
// team.rs 与 work/task.rs）→ HSM 拉起副核 → 本核 restore(run()) 进入首任务。
//
// 多核启动（hart B1）：hart 0 完整初始化（trap 栈/canary/spawn）后经 SBI HSM
// `hart_start` 拉起 hart 1..N-1；副核入口（_boot_entry）把 HSM 传入的
// a0=hartid / a1=opaque（= 本 hart trap 栈顶，寄存器传递免共享内存同步）装成
// tp/sp 后进入 boot_main → per-hart CSR 配置 → idle（spin+steal，见
// work::scheduler::idle_loop）。

use core::arch::global_asm;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::{block, frame};
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, SpaceBuilder};
use crate::putln;
use crate::runtime::trampoline::restore;
use crate::work::scheduler;
use crate::work::team::kernel;
use crate::work::{loader, team};

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

/// 启动多任务（阶段 A 线程模型）：spawn 演示团队后进入首个线程，永不返回。
///
/// Team1 = 双线程共享空间：首线程 a0=0 计数循环写 'A'（靠抢占轮转）；次线程
/// a0=1 写 'B' 后退出——验证「线程退出、团队/兄弟线程存活」的引用计数语义。
/// 之后是单线程团队回归：A counter（不自让出，靠抢占）→ B yielder（主动让出）
/// → C exiter（写 'C' 后退出）。S-timer 由 runtime::init 武装、trap_handler 内循环重武装。
pub fn init() -> ! {
    // per-hart 调度器状态按实际核数（DTB）动态分配——先于任何调度器访问
    scheduler::init();

    // PT 回收自测（debug）：unmap 时中间表必须当场归还——不泄漏、不 double-free
    #[cfg(debug_assertions)]
    pt_reclaim_selftest();

    // 记录内核持久帧基线：spawn 用户任务前的在途帧 + 内核堆支撑页。此后在途帧
    // 只应增用户任务所有 + 堆支撑页；关机时全部归还，由 tie::halt 的
    // check_baseline 断言零泄漏（见 frame.rs record_baseline/check_baseline）。
    #[cfg(debug_assertions)]
    frame::record_baseline();

    // 全部演示程序改为经 parser → loader → TaskBuilder 装载的**真 ELF**（user crate，
    // 静态链接于 USER_TEXT_BASE 0x10000），blob 装载 load_blob 随之移除。
    //
    // Team1「threader」：双线程共享同一地址空间——线程参数 a0 分支（0 → 'A' 循环、
    // 非 0 → 'B' 循环），多核下分布在两 hart 真实并行。先 Team 后 Task。
    // 全部演示任务（用户 + 内核）经统一 `Team::task()` 入口生成，错误一律 `?` 上抛至本边界。
    spawn_demos().expect("boot spawn failed");

    // 多核：HSM 启动副核（trap 栈/canary 已由 trap::init 就绪；副核 idle 后
    // 经 steal 从队列取活——任务即向各核迁移）
    boot_harts();

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走，见 scheduler::enter_first_task）
    putln!("task: entering first task");
    restore(scheduler::run())
}

/// 生成全部演示任务（用户 + 内核 ktask）；错误统一 `?` 上抛。返回前所有任务已入队。
fn spawn_demos() -> Result<(), MapError> {
    // Team1「threader」：双线程共享同一地址空间——线程参数 a0 分支 'A'/'B'。先 Team 后 Task。
    let (team1, entry1) = load_user(&include_bytes!("../../user/user-threader.elf")[..]);
    team1.task().name("thread-A").entry(entry1).arg(0).spawn()?;
    team1.task().name("thread-B").entry(entry1).arg(1).spawn()?;
    drop(team1); // 构造期句柄用完即弃——团队由它的线程持有

    // 单线程团队回归：counter/yielder/sleeper/exiter 行为不变
    for (elf, name) in [
        (
            &include_bytes!("../../user/user-counter.elf")[..],
            "counter",
        ),
        (
            &include_bytes!("../../user/user-yielder.elf")[..],
            "yielder",
        ),
        (
            &include_bytes!("../../user/user-sleeper.elf")[..],
            "sleeper",
        ),
        (&include_bytes!("../../user/user-exiter.elf")[..], "exiter"),
        (&include_bytes!("../../user/user-heaper.elf")[..], "heaper"),
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
    let parsed = crate::work::parser::parse(elf).expect("parse user elf");
    let space = SpaceBuilder::user().build().expect("space failed");
    let loaded = loader::load(space, elf, &parsed).expect("load user elf");
    let entry = loaded.entry;
    let team = team::TeamBuilder::new(loaded.space).spawn();
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
extern "C" fn boot_main() -> ! {
    let hart = machine::hart_id();
    putln!("hart {hart}: secondary boot, entering idle");
    crate::runtime::trap::init_hart();
    scheduler::idle()
}

/// PT 回收自测（debug）：map/unmap 循环验证中间表回收——无孤儿表、无 double-free。
///
/// 在 spawn 用户任务之前运行（分配器与 KERNEL_SPACE 均已就绪）。每轮：
/// map 4 MiB（4 KiB 页，根表槽 1）→ 表数 +3（1×L1 + 2×L0）；unmap → 回落；
/// 32 轮后「在途帧 − 堆支撑页」回到轮前（块堆缓存页不误报，口径同 check_baseline）。
#[cfg(debug_assertions)]
fn pt_reclaim_selftest() {
    const BASE: usize = 0x4000_0000; // 根表槽 1：堆窗口之后、栈窗口之前的空地
    const SIZE: usize = 4 * 1024 * 1024; // 4 MiB → 1×L1 + 2×L0
    const ROUNDS: usize = 32;

    let space = SpaceBuilder::user().build().expect("selftest: build space");
    let flags = PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
    let base_count = space.table_count();
    let held_before = frame::FRAME_ALLOCATOR
        .outstanding()
        .saturating_sub(block::live_pages());

    for round in 0..ROUNDS {
        // map：分配数据帧 + 中间表
        let mut frames = Vec::new();
        for _ in 0..(SIZE / PAGE_SIZE) {
            frames.push(
                Box::try_new_in([0u8; PAGE_SIZE], frame::allocator())
                    .expect("selftest: data frame"),
            );
        }
        let pa = PhysAddr::from_raw(frames[0].as_ptr() as usize);
        space
            .map(
                VirtAddr::from_raw(BASE),
                pa,
                SIZE,
                flags,
                MapKind::Anonymous,
                frames,
            )
            .expect("selftest: map");
        assert_eq!(
            space.table_count(),
            base_count + 3,
            "selftest round {round}: tables after map"
        );
        assert!(
            space.translate(VirtAddr::from_raw(BASE)).is_some(),
            "selftest round {round}: map hit"
        );

        // unmap：回收中间表 + 数据帧（树自底向上判空摘除；double-free 由分配器检测）
        space.unmap(VirtAddr::from_raw(BASE), SIZE);
        assert_eq!(
            space.table_count(),
            base_count,
            "selftest round {round}: tables after unmap"
        );
        assert!(
            space.translate(VirtAddr::from_raw(BASE)).is_none(),
            "selftest round {round}: unmap hit"
        );
    }

    let held_after = frame::FRAME_ALLOCATOR
        .outstanding()
        .saturating_sub(block::live_pages());
    assert_eq!(
        held_before, held_after,
        "selftest: net frames leaked: {held_before} → {held_after}"
    );
    drop(space);
    putln!("pt-reclaim selftest: ok ({ROUNDS} rounds, tables {base_count} → +3 → {base_count})");
}

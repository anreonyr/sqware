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
use alloc::vec::Vec;

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::{block, frame};
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{MapKind, SpaceBuilder};
use crate::putln;
use crate::runtime::trampoline::restore;
use crate::work::scheduler;
use crate::work::{loader, task, team};

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

    // Team1：双线程共享同一地址空间（两线程均长跑——多核下分布在两 hart 上
    // 真实并行：a0=0 → 'A' 循环，a0=1 → 'B' 循环）——先 Team 后任务
    let space1 = SpaceBuilder::user().build().expect("space1 failed");
    loader::load(&space1, program_d()).expect("load team1 failed");
    let team1 = team::TeamBuilder::new(space1).spawn();
    let _ = task::TaskBuilder::new(team1.clone())
        .name("thread-A")
        .arg(0)
        .spawn()
        .expect("spawn A failed");
    let _ = task::TaskBuilder::new(team1.clone())
        .name("thread-B")
        .arg(1)
        .spawn()
        .expect("spawn B failed");
    drop(team1); // 构造期句柄用完即弃——团队由它的线程持有
    // 单线程团队回归：A/B/C/E 行为不变（E 每 ~1.6s 写 'E' 后睡眠 1600 ms——
    // 任务级阻塞：Running → Blocked → deadline 堆 → unpark 唤醒）
    for (program, name) in [
        (program_a(), "counter"),
        (program_b(), "yielder"),
        (program_c(), "exiter"),
        (program_e(), "sleeper"),
    ] {
        let space = SpaceBuilder::user().build().expect("space failed");
        loader::load(&space, program).expect("load failed");
        let team = team::TeamBuilder::new(space).spawn();
        let _ = task::TaskBuilder::new(team)
            .name(name)
            .spawn()
            .expect("spawn failed");
    }

    // 多核：HSM 启动副核（trap 栈/canary 已由 trap::init 就绪；副核 idle 后
    // 经 steal 从队列取活——任务即向各核迁移）
    boot_harts();

    // 进入调度：从本 hart 调度器取首任务（不能用 spawn 返回的帧 PA——可能已被
    // 副核 steal 走，见 scheduler::enter_first_task）
    putln!("task: entering first task");
    restore(scheduler::run())
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
        let r = crate::ecall::HsmCall::new(crate::ecall::fid::Hsm::Start)
            .args(crate::ecall::scall::SArgs {
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

// ── 演示程序（手写机器码 blob，llvm-mc 核对字节；rv64gc 基础指令）──────────

/// A "counter"：每 262144 次迭代写 'A'（(t1 & 0x3ffff)==0），从不主动让出——
/// 靠定时器抢占切走。两个关键设计：
/// 1. 计数器用 **t1** 而非 a0：ENV_WRITE 的 a0 是返回值槽（帧恢复后 a0 = 字符
///    码），作为计数器会被破坏；
/// 2. andi 立即数仅 12 位有符号（0xfff 溢出、srli+andi 单级是 1/64 段选而非
///    点选），故用两级检查：低 11 位全零 && (t1>>11)&0x7f 全零 → 每 2^18 次。
///
/// 输出频率 ~12 字符/量子（0.1s），保持演示可读。
///
/// 布局（40 B）：addi t1,t1,1; andi t0,t1,0x7ff; bnez t0,+0x1c;
/// srli t0,t1,11; andi t0,t0,0x7f; bnez t0,+0x10;
/// li a7,1; li a0,'A'; ecall; j -0x24
const fn program_a() -> &'static [u8] {
    &[
        0x13, 0x03, 0x13, 0x00, // addi t1, t1, 1
        0x93, 0x72, 0xf3, 0x7f, // andi t0, t1, 0x7ff
        0x63, 0x9e, 0x02, 0x00, // bnez t0, +0x1c
        0x93, 0x52, 0xb3, 0x00, // srli t0, t1, 11
        0x93, 0xf2, 0xf2, 0x07, // andi t0, t0, 0x7f
        0x63, 0x98, 0x02, 0x00, // bnez t0, +0x10
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x10, 0x04, // li   a0, 0x41     ('A')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0xf0, 0xdf, 0xfd, // j    -0x24
    ]
}

/// B "yielder"：每次迭代主动让出（ENV_YIELD），每 4 次让出写 'B'。
/// B 每次运行只迭代 1 次（立即让出），跨运行累计 a0 计数——每 4 次运行
/// （~0.8s）输出一个 'B'，展示主动让出驱动的轮转。
///
/// 布局（36 B）：addi a0,a0,1; andi t0,a0,0x3; bnez t0,+0x10;
/// li a7,1; li a0,'B'; ecall; li a7,0; ecall; j -0x20
const fn program_b() -> &'static [u8] {
    &[
        0x13, 0x05, 0x15, 0x00, // addi a0, a0, 1
        0x93, 0x72, 0x35, 0x00, // andi t0, a0, 0x3
        0x63, 0x98, 0x02, 0x00, // bnez t0, +0x10
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x20, 0x04, // li   a0, 0x42     ('B')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x93, 0x08, 0x00, 0x00, // li   a7, 0        (ENV_YIELD)
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0xf0, 0x1f, 0xfe, // j    -0x20
    ]
}

/// C "exiter"：写 'C' 一次后退出（ENV_EXIT）。
///
/// 布局（20 B）：li a7,1; li a0,'C'; ecall; li a7,2; ecall; j 0（兜底）
const fn program_c() -> &'static [u8] {
    &[
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x30, 0x04, // li   a0, 0x43     ('C')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x93, 0x08, 0x20, 0x00, // li   a7, 2        (ENV_EXIT)
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0x00, 0x00, 0x00, // j    0（正常不可达兜底）
    ]
}

/// D "threader"：线程入口参数 a0 分支行为（多核线程模型 demo）。
///
/// a0 == 0 → 'A' 计数循环；a0 != 0 → 'B' 计数循环——同一空间双线程**均长跑**，
/// 多核下分布在两个 hart 上真实并行（同一 satp、不同 trap 帧）。
/// 计数用 t1 而非 a0：ENV_WRITE 的返回值槽是 a0，作为计数器会被破坏。
///
/// 布局（84 B）：beqz a0,+0x2c; [B 循环] addi t1,t1,1; andi t0,t1,0x7ff;
/// bnez t0,+0x1c; srli t0,t1,11; andi t0,t0,0x7f; bnez t0,+0x10;
/// li a7,1; li a0,'B'; ecall; j -0x24; [A 循环] addi t1,t1,1; andi t0,t1,0x7ff;
/// bnez t0,+0x1c; srli t0,t1,11; andi t0,t0,0x7f; bnez t0,+0x10;
/// li a7,1; li a0,'A'; ecall; j -0x24
const fn program_d() -> &'static [u8] {
    &[
        0x63, 0x06, 0x05, 0x02, // beqz a0, +0x2c    (a0==0 → 'A' 循环)
        // 'B' 循环（长跑）
        0x13, 0x03, 0x13, 0x00, // addi t1, t1, 1
        0x93, 0x72, 0xf3, 0x7f, // andi t0, t1, 0x7ff
        0x63, 0x9e, 0x02, 0x00, // bnez t0, +0x1c
        0x93, 0x52, 0xb3, 0x00, // srli t0, t1, 11
        0x93, 0xf2, 0xf2, 0x07, // andi t0, t0, 0x7f
        0x63, 0x98, 0x02, 0x00, // bnez t0, +0x10
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x20, 0x04, // li   a0, 0x42     ('B')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0xf0, 0xdf, 0xfd, // j    -0x24        (B 循环)
        // 'A' 循环（长跑）
        0x13, 0x03, 0x13, 0x00, // addi t1, t1, 1
        0x93, 0x72, 0xf3, 0x7f, // andi t0, t1, 0x7ff
        0x63, 0x9e, 0x02, 0x00, // bnez t0, +0x1c
        0x93, 0x52, 0xb3, 0x00, // srli t0, t1, 11
        0x93, 0xf2, 0xf2, 0x07, // andi t0, t0, 0x7f
        0x63, 0x98, 0x02, 0x00, // bnez t0, +0x10
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x10, 0x04, // li   a0, 0x41     ('A')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0xf0, 0xdf, 0xfd, // j    -0x24        (A 循环)
    ]
}

/// E "sleeper"：写 'E' 后睡眠 1600 ms（ENV_SLEEP 的毫秒语义）再循环——
/// 任务级阻塞演示：Running → Blocked（deadline 堆）→ Starved（unpark 唤醒）。
///
/// 布局（28 B）：li a7,1; li a0,'E'; ecall; li a7,4; li a0,1600; ecall; j -0x18
/// （字节经 llvm-mc 核对；ENV_SLEEP = 4，a0 = 毫秒）
const fn program_e() -> &'static [u8] {
    &[
        0x93, 0x08, 0x10, 0x00, // li   a7, 1        (ENV_WRITE)
        0x13, 0x05, 0x50, 0x04, // li   a0, 0x45     ('E')
        0x73, 0x00, 0x00, 0x00, // ecall
        0x93, 0x08, 0x40, 0x00, // li   a7, 4        (ENV_SLEEP)
        0x13, 0x05, 0x40, 0x06, // li   a0, 1600     (ms)
        0x73, 0x00, 0x00, 0x00, // ecall
        0x6f, 0xf0, 0x9f, 0xfe, // j    -0x18        (28 B：0x18 + (-0x18) = 0x00)
    ]
}

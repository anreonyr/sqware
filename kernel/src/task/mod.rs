// 任务（task）— 可调度单元：独立地址空间 + 用户栈 + trap 帧 + 调度状态
//
// 阶段 C：多任务内核。每个 Task 持有独立 Space（各自 ASID + TRAP_CONTEXT 帧），
// 由 S-timer 抢占 + envcall 驱动切换。切换完全走 trap 链路——trap_handler 返回
// 下一任务帧 → __restore 切 satp + sret，无独立切换汇编（见 runtime/trampoline.rs）。
//
// 子模块：
//   scheduler — round-robin 调度器（队列/切换/回收/当前空间借出）
//   envcall   — 用户态环境调用 ABI（RISC-V "Environment Call"，见 riscv crate
//               的 Exception::UserEnvCall 命名——术语与规范同源）
//
// 阶段 B 的 user.rs 被本模块吸收：USER_SPACE 单例 → per-task Space；boot() →
// init()；trap.rs 缺页路由改经 task::with_current_space 取当前空间。

pub mod envcall;
pub mod scheduler;

use crate::memory::manager::addr::VirtAddr;
use crate::putln;
use crate::runtime::trampoline::restore;

/// 用户程序加载基址（阶段 B 沿用；阶段 C 后续 ELF 加载亦用此基址）。
pub const USER_TEXT_BASE: VirtAddr = VirtAddr::from_raw(0x1_0000);

// 常用 API 收敛到 task::（scheduler 内部实现，详见 scheduler.rs）
pub use scheduler::{exit_current, spawn, tick, with_current_space};

/// 启动多任务（阶段 C）：spawn 演示任务后进入首个任务，永不返回。
///
/// 顺序：A counter（不自让出，靠抢占）→ B yielder（主动让出）→ C exiter
/// （写 'C' 后退出）。S-timer 由 runtime::init 武装、trap_handler 内循环重武装。
pub fn init() -> ! {
    let first = scheduler::spawn(program_a(), "counter").expect("spawn A failed");
    let _ = scheduler::spawn(program_b(), "yielder").expect("spawn B failed");
    let _ = scheduler::spawn(program_c(), "exiter").expect("spawn C failed");
    putln!("task: entering first task");
    restore(first.as_usize())
}

// ── 演示程序（手写机器码 blob，llvm-mc 核对字节；rv64gc 基础指令）──────────

/// A "counter"：每 262144 次迭代写 'A'（(t1 & 0x3ffff)==0），从不主动让出——
/// 靠定时器抢占切走。两个关键设计：
/// 1. 计数器用 **t1** 而非 a0：ENV_WRITE 的 a0 是返回值槽（帧恢复后 a0 = 字符
///    码），作为计数器会被破坏；
/// 2. andi 立即数仅 12 位有符号（0xfff 溢出、srli+andi 单级是 1/64 段选而非
///    点选），故用两级检查：低 11 位全零 && (t1>>11)&0x7f 全零 → 每 2^18 次。
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

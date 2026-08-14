// 锁模块 — 中断安全同步原语集合
//
// 维度一（互斥 vs 惰性）：
//   - 互斥（可反复读写）：SpinLock / BareLock / RwLock / RelLock
//   - 惰性（写一次读多次，读取无锁）：OnceLock / LazyLock
//
// 维度二（中断安全）：
//   - SpinLock / RwLock / RelLock：获取时关闭 sstatus.SIE，可从中断上下文安全获取
//   - BareLock：不关中断，仅供任务上下文调用（lock() 为 unsafe fn）
//   - OnceLock / LazyLock：读路径无锁，无中断安全问题
//
// 多核准备：所有互斥 guard 携带 !Send 标记（锁须在本 hart 释放）；
// 关中断逻辑统一由 trap::TrapGuard 提供；RelLock 通过 hal::cpu::hart_id 区分持有者。
//
// panic 路径：panic.rs 经 sink::SBI_WRITER（SBI ecall）无锁直写控制台，故意绕过所有锁，新框架不影响。
//
// # Lock hierarchy
//
// To prevent deadlocks, locks must be acquired in the following order:
//
//   1. KERNEL_SPACE        (RelLock)   — 内核地址空间
//   2. Space.inner  (RelLock)   — 任务地址空间可变状态（页表读写 + regions/frames/heap）
//   3. ASID_ALLOCATOR      (SpinLock)  — ASID 分配器
//   4. FRAME_ALLOCATOR     (SpinLock)  — 物理帧分配器（buddy）
//   5. portal / block      (SpinLock / TrapGuard) — 全局堆分配
//
// A lock at level N may be acquired while holding a lock at level < N.
// Acquiring a lock at level N while holding one at level ≥ N is forbidden.
// OnceLock / LazyLock read paths are lock-free and exempt from this hierarchy.
//
// 关键嵌套边：Space.inner → FRAME_ALLOCATOR（map/page_fault 持空间锁分配帧）。
// 用户空间构建（SpaceBuilder::user().build()）中 ASID → KERNEL_SPACE 为顺序获取（drop 前一把再拿后一把），不嵌套。
// （hub::devices / INTERRUPT_HANDLERS 为规划中模块，接入后插入对应层级。）

mod bare;
mod dep;
mod lazy;
mod once;
pub(crate) mod reentrant;
mod rw;
mod spin;
mod trap;
pub(crate) use trap::TrapGuard;

#[allow(unused_imports)]
// BareLock：锁体系原语，当前无用户（platform::PROBE_ERROR 移除后），预留
pub use bare::BareLock;
// LazyLock 可用但暂未使用：crate::lock::lazy::LazyLock
pub use once::OnceLock;
pub use reentrant::RelLock;
pub use rw::RwLock;
pub use spin::SpinLock;

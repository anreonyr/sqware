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
// 关中断逻辑统一由 trap::TrapGuard 提供；RelLock 通过 machine::hart_id 区分持有者。
//
// panic 路径：halt.rs 的 panic_handler 经 console::_write 无锁直写控制台，故意绕过所有锁。
//
// # Lock hierarchy
//
// To prevent deadlocks, locks must be acquired in the following order:
//
//   1. SCHEDULERS[hart]   (SpinLock)  — 每 hart 调度器：running + starved（task::scheduler）
//                                          （类型级：不同 hart 的锁彼此不嵌套；
//                                          steal 走 try_lock（非阻塞），无锁序依赖）
//   2. Space.inner  (RelLock)   — 任务地址空间可变状态（Durable：页表/常数映射 + dynamic：窗口）
//                                   （内核空间同属此级：唯一归属 KERNEL_TEAM.space，
//                                   RelLock 可重入——持锁缺页时同 hart 再入不死锁）
//   3. Team.tasks          (SpinLock)  — 团队成员簿记（弱引用列表；纯 Vec 操作，
//                                          **与 Space.inner 禁止嵌套持有**——
//                                          push_task/prune_tasks 锁内绝不调 space 方法）
//   3. TIMER_DEADLINES     (SpinLock)  — 计时模块的 deadline 堆（runtime::time 段二）：
//                                         与 blocked/reaped 同级；park 路径 1 → 3
//                                         嵌套合法（blocked 与 timer 顺序获取、不嵌套）
//   3. blocked / reaped    (SpinLock)  — 全局容器（task::scheduler）：Blocked 为
//                                          handle→Task 睡眠映射 / Reaped 回收队列：
//                                         park 路径 1 → 3 嵌套合法；unpark 路径**先放
//                                         堆锁/队列锁再取调度锁**——绝不持队列锁取
//                                         调度锁（防 ABBA）
//   4. ASID_ALLOCATOR      (SpinLock)  — ASID 分配器
//   5. FRAME_ALLOCATOR     (SpinLock)  — 物理帧分配器（frame）
//   6. block inner / pump  (SpinLock)  — 每池实例锁（互不嵌套靠路由纪律，保持
//                                        exempt；见 depend::Level::Block）
//   7. LEDGER              (SpinLock)  — 护栏账本（fence::ledger；持锁绝不分配，
//                                        audit 只读块归属不受限——tally 更高）
//   8. tally               (SpinLock)  — block 簿记表（全部表访问自锁：own 单独持、
//                                        池内路径 inner → tally、审计 ledger →
//                                        tally 只读；tally 是叶锁，无反向边）
//   9. spare               (SpinLock)  — 后备仓（常态显式调用 + 崩溃无锁切换；
//                                        常驻 ring 在 boot 无锁期分配）
//   portal                (无锁)      — 原子后端模式判别（AtomicU8，Backend），
//                                        不取任何锁，不在层级中
//
// A lock at level N may be acquired while holding a lock at level < N.
// Acquiring a lock at level N while holding one at level ≥ N is forbidden.
// OnceLock / LazyLock read paths are lock-free and exempt from this hierarchy.
//
// 关键嵌套边：Space.inner → FRAME_ALLOCATOR（map/page_fault 持空间锁分配帧）；
// SCHEDULERS[hart] → Team.tasks（spawn 入簿 / exit 清理，1 → 3）；
// SCHEDULERS[hart] → Space.inner（reap 锁内回收，1 → 2 → 5）；
// SCHEDULERS[hart] → TIMER_DEADLINES / blocked（park：reserve/入簿/arm_at 顺序
// 获取、不嵌套，1 → 3）；Team.tasks 与 Space.inner 只顺序获取、永不嵌套。
// 用户空间构建（SpaceBuilder::user().build()）中 ASID 与内核 Space（seed 读 trampoline 叶）
// 为顺序获取（drop 前一把再拿后一把），不嵌套。
// per-hart trap 栈的分配发生在 boot（无锁需求）。

mod bare;
mod depend;
mod lazy;
mod once;
pub(crate) mod reentrant;
mod rw;
mod spin;
mod trap;
pub(crate) use trap::TrapGuard;

#[allow(unused_imports)]
// BareLock：锁体系原语，当前无用户，预留
pub use bare::BareLock;
// LazyLock 可用但暂未使用：crate::lock::lazy::LazyLock
pub use once::OnceLock;
pub use reentrant::RelLock;
// RwLock：锁体系原语，当前无用户，保留 re-export
/// 锁层级（depend 具名化；参与锁用 new_level 声明；None = exempt）。
pub use depend::Level;
/// 分配点返回地址捕获（integrity alloc-site 与诊断报告用）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) use depend::ra;
#[allow(unused_imports)]
pub use rw::RwLock;
pub use spin::SpinLock;
/// 注入地址符号化回调（boot 装配后调用；未注入则诊断打印裸地址）。
pub use crate::runtime::diagnose::addr::set_symbolizer;

/// debug 装配 lockdep（release 为 no-op；boot 分配器就绪后调用一次）。
#[cfg(debug_assertions)]
pub fn init_depend(hart_count: usize) -> Result<(), depend::DepInitError> {
    depend::init(hart_count)
}

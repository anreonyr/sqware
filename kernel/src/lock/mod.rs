// 锁模块 — 中断安全同步原语集合。
//
// 互斥（SpinLock / BareLock / RwLock / RelLock）vs 惰性（OnceLock / LazyLock）。
// SpinLock / RwLock / RelLock 获取时关闭 sstatus.SIE，可从中断上下文安全获取；
// BareLock 不关中断，仅任务上下文（lock() 为 unsafe fn）；OnceLock / LazyLock
// 读路径无锁。
//
// 锁层级（升序获取、禁止反向嵌套；OnceLock/LazyLock 读路径豁免，层级见 depend::Level）：
//   1. SCHEDULERS[hart]
//   2. Space.inner
//   3. Team.tasks / TIMER_DEADLINES / blocked / reaped
//   4. ASID_ALLOCATOR
//   5. FRAME_ALLOCATOR
//   6. block inner / pump
//   7. LEDGER
//   8. tally
//   9. spare

#![allow(unused)]

mod bare;
mod depend;
mod lazy;
mod once;
pub(crate) mod reentrant;
mod rw;
mod spin;
mod trap;

// ── depend 钩子宏族（不可重入独占锁入口样板归一）────────────────────────
// 读调用点 + 层级校验 / 记入 / 移除三段样板收进宏；宏体内部 cfg(debug_assertions)
// ——release 展开为空，锁退回纯互斥原语。可重入锁（RelLock）持有自身计数语义
// 不套用（其 check/acquire/release 有 owner 条件，见 reentrant.rs）。
// 记入为**全锁记账**（exempt 也记，level=None）：acquire/release 双侧平衡 +
// contains 全覆盖；层级校验由 depend 内部裁决（只看 Some）。

/// 锁入口样板：读调用者返回地址（入口第一件事——任何函数调用都会覆盖 ra；
/// 与 check 分离：check 必须在关中断（TrapGuard::save）之后调用，宏内合并会
/// 把 held set 访问提前到 SIE 开态，违反「仅 SIE 关时读写」纪律）。
macro_rules! depend_enter {
    ($lock:expr) => {{
        let caller: usize;
        // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
        unsafe { core::arch::asm!("mv {0}, ra", out(reg) caller) };
        caller
    }};
}

/// 自旋前校验（调用方在关中断后调；addr 自取）。**重入检测（contains）与
/// level 解耦**——exempt（level=None）锁也查重入（恢复原锁内 owner 检测的
/// 覆盖），层级校验仅对参与锁生效。
macro_rules! depend_check {
    ($lock:expr, $caller:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::check(
            $lock as *const _ as *const () as usize,
            $lock.level,
            $caller,
        );
    }};
}

/// 获取成功后记入持有集（Held 顺带记本次调用点）。**exempt 锁（level=None）
/// 也记入**（None 槽位）——与 `depend_release!` 双侧平衡（否则 release 对
/// 未记账锁必然误报 unheld），并让 contains 对 exempt 锁的重入检测生效；
/// 层级校验只看 Some（见 depend::acquire）。
macro_rules! depend_acquire {
    ($lock:expr, $caller:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::acquire(
            $lock as *const _ as *const () as usize,
            $lock.level,
            $caller,
        );
    }};
}

/// guard Drop 移除持有集（`$lock` 为锁引用，如 `self.lock`；level 不参与匹配）。
macro_rules! depend_release {
    ($lock:expr) => {{
        #[cfg(debug_assertions)]
        crate::lock::depend::release($lock as *const _ as *const () as usize);
    }};
}
pub(crate) use depend_acquire;
pub(crate) use depend_check;
pub(crate) use depend_enter;
pub(crate) use depend_release;

#[allow(unused_imports)]
// BareLock：锁体系原语，当前无用户，预留
pub use bare::BareLock;
// LazyLock 可用但暂未使用：crate::lock::lazy::LazyLock
pub use once::OnceLock;
pub use reentrant::RelLock;
// RwLock：锁体系原语，当前无用户，保留 re-export
/// 锁层级（depend 具名化；参与锁用 new_level 声明；None = exempt）。
pub use depend::Level;
#[allow(unused_imports)]
pub use rw::RwLock;
pub use spin::SpinLock;

/// debug 装配 lockdep（release 为 no-op；boot 分配器就绪后调用一次）。
#[cfg(debug_assertions)]
pub fn init_depend(hart_count: usize) -> Result<(), depend::DepInitError> {
    depend::init(hart_count)
}

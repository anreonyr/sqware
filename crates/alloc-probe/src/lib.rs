//! alloc-probe — 把内核真实分配器**机制包**搬到宿主上单独压测。
//!
//! # 为什么要有这个 crate（解耦）
//!
//! 分配器自己的账坏了，与"某个内核对象没析构"在读数上曾经长得一模一样。
//! 本 crate 只压前者：`kernel-allocator` 的 Bump/Frame/Block/Hybrid/Spare 在这里装台运行，
//! 分配器元数据落在宿主堆上，泄漏检测器能直接看见"分配器自己有没有漏掉自己的簿记"；
//! 内核对象的生命周期不在场 ⇒ 任何失败都只能归因于分配器。
//!
//! 宿主侧只保留**有意义的适配**：台面、线程即 hart 的夹具、宿主堆锁实现、统计计数。
//! 生产算法只有一份，住在 `kernel-allocator`；这里不 `include!` 内核源码，也不复制算法。
//!
//! # 判据分三层 + 一个未接线的出口（见 README）
//!
//! | 层 | 通道 | 判什么 |
//! |---|---|---|
//! | ① 后端（**三选一**，见本文件接管段） | `smartalloc` / `mockalloc` / `dhat` | 孤儿块 / 分配释放**配对** / 宿主堆**用量** |
//! | ①′ 零依赖后端 | `RUSTFLAGS=-Zsanitizer=leak`（**必须不接管**） | LeakSanitizer 看宿主堆 |
//! | ② 影子账（`harness` + `tests.rs`） | 本 crate 自己 | 区间不重叠 / 指针满足对齐 / 交付区写读一致 / `BlockAllocator::allocate` 的请求门必须拒 |
//! | ③ 随机序列（`tests/{pairing,heap}.rs` + `harness::plan`） | proptest | 把②的判据铺到随机分配/释放序列与随机工作负载上 |
//! | ④ 当场炸的通道（[`fault::allocator_fault`]） | **未接线**（照实记：零调用点） | 分配器**自身**违例本应由它当场炸；目前实际走②的 `Violation`（`Err`）归因 |
//!
//! ②③ 两条腿**不依赖后端**：`plain` / `smartalloc` / `mockalloc` / `dhat` 四档都跑同一批判据，
//! 后端只换读数口径。

#![feature(allocator_api)]
#![allow(dead_code, unused_imports, unused_variables)]

extern crate alloc;

/// 宿主侧的锁与一次性槽。锁仍是既有的 `shim_lock::SpinLock<()>` 包 `std::sync::Mutex`；
/// 它实现机制包的 `Lock`，返回原生 unit guard，不把状态挂到锁上。
mod shim_lock {
    use core::ops::DerefMut;
    use std::sync::Mutex;

    /// 锁层级（宿主上只作标记：与内核同角色的层级值保持一致）。
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub enum Level {
        Scheduler = 1,
        Space = 2,
        L3 = 4,
        Asid = 5,
        Frame = 6,
        Block = 7,
        Ledger = 8,
        Tally = 9,
        Spare = 10,
    }

    pub struct SpinLock<T> {
        v: Mutex<T>,
        level: Level,
    }

    // SAFETY: 宿主用例由 `.cargo/config.toml` 的 `RUST_TEST_THREADS=1` 约束；
    // 锁本身仍是 std 的互斥量，`T` 的访问只经过 `MutexGuard`。
    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(v: T) -> Self {
            Self {
                v: Mutex::new(v),
                level: Level::Frame,
            }
        }

        pub const fn new_level(level: Level, v: T) -> Self {
            Self {
                v: Mutex::new(v),
                level,
            }
        }

        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.v.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    // 机制包只认原生 unit guard；这里直接返回既有本地锁的原生 MutexGuard。
    unsafe impl kernel_allocator::Lock for SpinLock<()> {
        type Guard<'a>
            = std::sync::MutexGuard<'a, ()>
        where
            Self: 'a;

        fn new(role: Option<kernel_allocator::LockRole>) -> Self {
            match role {
                Some(kernel_allocator::LockRole::Frame) => SpinLock::new_level(Level::Frame, ()),
                Some(kernel_allocator::LockRole::Tally) => SpinLock::new_level(Level::Tally, ()),
                Some(kernel_allocator::LockRole::Spare) => SpinLock::new_level(Level::Spare, ()),
                None => SpinLock::new(()),
            }
        }

        fn lock(&self) -> Self::Guard<'_> {
            SpinLock::lock(self)
        }
    }

    /// 一次性初始化槽（宿主上直接用 `std::sync::OnceLock`）。
    pub struct OnceLock<T> {
        v: std::sync::OnceLock<T>,
    }

    // SAFETY: 同上（单线程用例；std 的 `OnceLock` 只多要 `T: Send + Sync`）。
    unsafe impl<T> Send for OnceLock<T> {}
    unsafe impl<T> Sync for OnceLock<T> {}

    impl<T> OnceLock<T> {
        pub const fn new() -> Self {
            Self {
                v: std::sync::OnceLock::new(),
            }
        }
        pub fn get(&self) -> Option<&T> {
            self.v.get()
        }
        pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
            self.v.get_or_init(f)
        }
        pub fn set(&self, v: T) -> Result<(), T> {
            self.v.set(v)
        }
    }
}

/// 宿主机器事实与核身份夹具：一块宿主缓冲冒充物理内存，线程即 hart。
pub mod fixture {
    use super::shim_lock::OnceLock;
    use kernel_allocator::Region;

    pub(crate) const MAX_RESERVED: usize = 2;

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Machine {
        pub free: Region,
        pub reserved: [Option<Region>; MAX_RESERVED],
    }

    static INFO: OnceLock<Machine> = OnceLock::new();
    static HARTS: OnceLock<usize> = OnceLock::new();

    /// 装台：把一块宿主缓冲当作"物理内存"交给分配器。**必须在 init 之前调一次。**
    pub(crate) fn configure(base: usize, size: usize, harts: usize) {
        let _ = INFO.set(Machine {
            free: Region::new(base, size),
            reserved: [None; MAX_RESERVED],
        });
        let _ = HARTS.set(harts);
    }

    pub(crate) fn info() -> &'static Machine {
        INFO.get()
            .expect("machine not configured (call configure first)")
    }

    pub fn hart_count() -> usize {
        *HARTS.get().unwrap_or(&4)
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct HartId(usize);

    impl HartId {
        pub const fn new(id: usize) -> Self {
            Self(id)
        }

        pub const fn get(self) -> usize {
            self.0
        }

        pub const fn bit(self) -> (usize, usize) {
            (
                self.0 / usize::BITS as usize,
                1usize << (self.0 % usize::BITS as usize),
            )
        }
    }

    thread_local! {
        static HART: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
    }
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    /// 当前 hart（宿主上 = 当前线程）；块池按它选 per-hart 池。
    pub(crate) fn hart_id() -> HartId {
        HART.with(|c| {
            HartId::new(c.get().unwrap_or_else(|| {
                let n = hart_count();
                let id = if n == 0 {
                    0
                } else {
                    NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed) % n
                };
                c.set(Some(id));
                id
            }))
        })
    }

    /// **宿主专用**：把当前线程绑到指定 hart，让 per-hart 池归属可确定断言。
    pub fn hart_bind(id: usize) {
        HART.with(|c| c.set(Some(id)));
    }
}

/// 宿主侧的实际机制实例：装台槽与 effect 在 `host` 子模块里，算法在 `kernel-allocator`。
/// 对外只留 `host::{block,frame}::allocator()` 与 `fixture` 的 hart 夹具。
pub mod host;

// ── 判据：分配器自身违例的宿主通道（**未接线**：照实记，当前零调用点）────────

pub mod fault {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static FAULTS: AtomicUsize = AtomicUsize::new(0);

    /// 分配器**自身**的违例：与"对象泄漏"无关，故在这里当场炸。
    ///
    /// **未接线**（照实记）：本 crate 里**零调用点** —— 分配器自身的违例目前一律由影子账的
    /// `Violation`（`Err`）归因。这个出口留着，但没有接上。
    #[cold]
    pub fn allocator_fault(msg: core::fmt::Arguments<'_>) -> ! {
        FAULTS.fetch_add(1, Ordering::Relaxed);
        panic!("[alloc-probe] AllocatorFault: {msg}");
    }
}

pub use fault::FAULTS as allocator_faults;

/// 内核的两个打印宏（`crate::putln!` / `crate::tag!`）：宿主上分别落到 stdout 与恒等。
#[macro_export]
macro_rules! putln {
    ($($t:tt)*) => {{ println!($($t)*); }};
}

/// `tag!(Prime, expr)`：宿主上类目观测不建表，标签是恒等。
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {
        $v
    };
}

// ── 宿主堆的接管：三个后端**三选一**（**debug 档**）────────────────────────────
//
// rustc 只允许一个 `#[global_allocator]`，而这三个后端的读数口径互不相同，故**三选一**；
// `.cargo/config.toml` 的别名各给一条路，`compile_error!` 把"同时开两个"挡在编译期：
//
//   feature      全局分配器                     读数
//   smartalloc   smartalloc::SmartAlloc         收尾 `dump_orphans()` 点名还在账上的块
//   mockalloc    mockalloc::Mockalloc<System>   `record_allocs` 圈出的"一幕"里判配对（`tests/pairing.rs`）
//   dhat         dhat::Alloc                    `HeapStats` 的 total/curr/max 块数与字节数（`tests/heap.rs`）
//
// 三者都只在 **debug 档**存在：`smartalloc` 自己写着 `#![cfg(debug_assertions)]`；
// 另两条腿的判据（装台预算、每幕零增长）也只在 debug 档语义下成立（测试跑的就是 debug）。
#[cfg(any(
    all(feature = "smartalloc", feature = "mockalloc"),
    all(feature = "smartalloc", feature = "dhat"),
    all(feature = "mockalloc", feature = "dhat"),
))]
compile_error!(
    "宿主堆只能由一个后端接管：`smartalloc` / `mockalloc` / `dhat` **三选一**（见 README「六条路」与 `.cargo/config.toml` 的别名）"
);

/// **接管点甲**：debug 档（且带 `smartalloc` feature）的宿主堆从这里过。
#[cfg(all(feature = "smartalloc", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: smartalloc::SmartAlloc = smartalloc::SmartAlloc;

/// **接管点乙**：debug 档（且带 `mockalloc` feature）的宿主堆从这里过。
#[cfg(all(feature = "mockalloc", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: mockalloc::Mockalloc<std::alloc::System> =
    mockalloc::Mockalloc(std::alloc::System);

/// **接管点丙**：debug 档（且带 `dhat` feature）的宿主堆从这里过。
#[cfg(all(feature = "dhat", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: dhat::Alloc = dhat::Alloc;

/// 收尾转储：把此刻仍在 smartalloc 账上的缓冲（"孤儿缓冲"）打出来。
///
/// 未接管时（`--no-default-features` 或 release）是 no-op。
pub fn dump_orphans() {
    #[cfg(all(feature = "smartalloc", debug_assertions))]
    smartalloc::sm_dump(true);
}

/// **台面与模型**：第一层两条腿（`tests/pairing.rs` / `tests/heap.rs`）与 `tests.rs` 共用的
/// 被压测对象与判据。`pub` 是给**集成测试**用的（那是独立 crate，看不见 `#[cfg(test)]` 模块）。
pub mod harness;

/// **并发台面**：线程即 hart，判据在 `tests/mt.rs`。
pub mod concurrent;

#[cfg(test)]
mod tests;

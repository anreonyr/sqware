//! alloc-probe — 把内核**真实**的帧/块分配器源码搬到宿主上单独压测。
//!
//! # 为什么要有这个 crate（解耦）
//!
//! 内核里"分配器的账坏了"与"某个内核对象没析构"在读数上曾经长得一模一样：都表现为
//! 关机审计里一块内存还在册（那一层审计已按用户裁决整体删除，见 `docs/memory.md` §5）。
//! 两者的**修法完全不同** —— 前者在分配器内部（表/链/簿记三套口径对不上），后者在对象
//! 的持有者（谁没 drop）。本 crate 是测试侧的同一刀：
//!
//! * 把 `frame.rs` / `block.rs` / `bump.rs`（**逐字未改的内核源码**，按 `include!`
//!   引进来）放到宿主上跑；
//! * 分配器**元数据**的分配落在宿主堆上 ⇒ 泄漏检测器（LeakSanitizer / smartalloc）
//!   能直接看见"分配器自己有没有漏掉它自己的簿记"；
//! * 内核对象的生命周期**不在场** ⇒ 任何失败都只能归因于分配器；
//! * 宿主侧的判据是**影子账**（`tests.rs`：同一帧不得交付两次、块池区间不得重叠）与
//!   [`fault::allocator_fault`] 这条当场炸的通道 —— 内核侧的对应判据是框架档的用例
//!   （`health/{pagetable,spare,stress}.rs`），两边不共享代码，只共享同一批不变量。
//!
//! 两种泄漏检测后端（互斥，见 README）：`RUSTFLAGS=-Zsanitizer=leak`（默认路径，零依赖）
//! 与 `--features smartalloc`。

#![feature(allocator_api)] // 内核分配器源码用 `core::alloc::Allocator`（bump.rs）
#![allow(dead_code, unused_imports, unused_variables)]

extern crate alloc;

// ── 平台垫片：内核里这些来自 `crate::{lock, machine, memory}` ──────────────

pub mod lock {
    use core::ops::DerefMut;
    use std::sync::Mutex;

    /// 锁层级（内核里由 lockdep 用；宿主上只作标记）。
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

    // SAFETY: 宿主用例是单线程的（每个用例各自建台），锁只用来对齐内核的类型形状。
    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(v: T) -> Self {
            Self { v: Mutex::new(v), level: Level::Frame }
        }
        pub const fn new_level(level: Level, v: T) -> Self {
            Self { v: Mutex::new(v), level }
        }
        pub fn lock(&self) -> impl DerefMut<Target = T> + '_ {
            self.v.lock().unwrap_or_else(|e| e.into_inner())
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
            Self { v: std::sync::OnceLock::new() }
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

pub mod machine {
    use super::lock::OnceLock;

    pub const MAX_RESERVED: usize = 2;

    #[derive(Clone, Copy, Debug)]
    pub struct Region {
        pub base: usize,
        pub size: usize,
    }

    #[derive(Clone, Copy, Debug)]
    pub struct Machine {
        /// 空闲 DRAM 区（本 crate 用一块对齐好的宿主缓冲冒充）。
        pub free: Region,
        /// 保留区（initrd / 设备树）——测试里留空。
        pub reserved: [Option<Region>; MAX_RESERVED],
    }

    static INFO: OnceLock<Machine> = OnceLock::new();
    static HARTS: OnceLock<usize> = OnceLock::new();

    /// 装台：把一块宿主缓冲当作"物理内存"交给分配器。**必须在 init 之前调一次。**
    pub fn configure(base: usize, size: usize, harts: usize) {
        let _ = INFO.set(Machine {
            free: Region { base, size },
            reserved: [None; MAX_RESERVED],
        });
        let _ = HARTS.set(harts);
    }

    pub fn info() -> &'static Machine {
        INFO.get().expect("machine not configured (call configure first)")
    }

    pub fn hart_count() -> usize {
        *HARTS.get().unwrap_or(&4)
    }

    /// 当前 hart（宿主单线程 ⇒ 恒 0）。内核用它选 per-hart 池。
    pub fn hart_id() -> usize {
        0
    }
}

pub mod memory {
    pub const PAGE_SIZE: usize = 0x1000;

    pub mod allocator {
        use core::ptr::NonNull;

        // 内核源码**逐字未改**地编进来（`include!` 的路径相对本文件 ⇒ 三级上到仓库根）。
        // 用 `include!` 而不是 `#[path]`：后者对嵌套模块按"模块目录"解析，层级会绕。
        pub mod bump {
            include!("../../../kernel/src/memory/allocator/bump.rs");
        }
        pub mod frame {
            include!("../../../kernel/src/memory/allocator/frame.rs");
        }
        pub mod block {
            include!("../../../kernel/src/memory/allocator/block.rs");
        }

        /// 统计出口的宿主版：**只是计数**，不参与分配器正确性判据。
        ///
        /// 与内核 `statistics.rs` 同名同形（内核那份的读侧是四个判据输入，宿主侧由
        /// 影子账自己算，故这里只留写侧的四个 `record_*`）。
        pub mod statistics {
            use std::sync::atomic::{AtomicUsize, Ordering};

            pub static FRAME_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static FRAME_GIVE: AtomicUsize = AtomicUsize::new(0);
            pub static POOL_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static POOL_GIVE: AtomicUsize = AtomicUsize::new(0);

            pub fn init() {}

            pub fn record_frame_take() {
                FRAME_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_frame_give() {
                FRAME_GIVE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_pool_take() {
                POOL_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_pool_give() {
                POOL_GIVE.fetch_add(1, Ordering::Relaxed);
            }
        }

        /// 分配器初始化错误（与内核 `allocator/mod.rs` 同名同义；内核那份用
        /// `fack::Error` 派生，宿主这份手写，免得为一个 derive 多拉一个依赖）。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum InitError {
            NoFreeMemory,
            OutOfMemory,
            NoFreeFrames,
            NoHarts,
            AlreadyInitialized,
        }

        /// 与内核同名同形：`erra::Result`（带调用点上下文的 `Result`）。
        pub type InitResult<T> = erra::Result<T, InitError>;

        /// 自由链节点（内核 `Link` 的宿主版：同字段、同可见性规则）。
        pub(crate) struct Link {
            pub(crate) prev: Option<NonNull<Link>>,
            pub(crate) next: Option<NonNull<Link>>,
        }

        impl Link {
            pub(crate) fn new(prev: Option<NonNull<Link>>, next: Option<NonNull<Link>>) -> Self {
                Self { prev, next }
            }
        }
    }
}

// ── 判据：分配器自身违例的宿主通道（当场炸，不记账）──────────────────────

pub mod fault {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static FAULTS: AtomicUsize = AtomicUsize::new(0);

    /// 分配器**自身**的违例：与"对象泄漏"无关，故在这里当场炸。
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

/// `tag!(Prime, expr)`：内核里给块打来源标签（`fence` 删除后内核那份已是恒等宏）。
#[macro_export]
macro_rules! tag {
    ($kind:ident, $v:expr) => {
        $v
    };
}

// ── 宿主堆的接管：`smartalloc` 当 `#[global_allocator]`（**debug 档**）──────────
//
// 就这一行 —— crate 原样用它自己的 `SmartAlloc`，不再包任何自定义分配器：
//
//     #[global_allocator]
//     static HOST: smartalloc::SmartAlloc = smartalloc::SmartAlloc;
//
// 于是本 crate 的**每一次**宿主分配（含被压测的内核分配器自己去要的元数据）都记在
// smartalloc 的账上；收尾 `dump_orphans()` 把还在账上的块连**分配点**一起打出来。
//
// 为什么只可能是 debug 档：`smartalloc` 自己写着 `#![cfg(debug_assertions)]` ——
// release 下整个 crate 是空的（连 `SmartAlloc` 类型都不存在）。测试跑的就是 debug 档。
//
// # 两个必须交代的前提（都在本 crate 里写明并落实，而不是靠"应该没事"）
//
// ① **指针对齐**：上游 `sizeof(struct abufhead) == 40`（x86-64），用户指针 = malloc 基址
//    + 40 ⇒ 只有 8 字节对齐，违反 Rust `GlobalAlloc` 的契约（返回指针须满足
//    `layout.align()`）。直接挂上去的实测症状：进程起始阶段 SIGSEGV，`movdqa (%r14)`
//    —— hashbrown 的表扩容里第一条 16 字节 SIMD 载入，一个用例都跑不到。
//    修法见 `Cargo.toml` 的 `[patch.crates-io]`：本地 vendored `smartalloc-sys` 把
//    `SM_ALIGN = 64` 写进 C（`smalloc` 抬高基址并把真基址记在头里），**Rust 侧照用 crate**。
// ② **单线程**：C 层是一张无锁全局链表 + `assert`，多线程并发 alloc/free 会把它写坏。
//    故 `.cargo/config.toml` 里钉 `RUST_TEST_THREADS = "1"`（crate 自己的使用前提）。
//
// 与 `-Zsanitizer=leak` 互斥：接管之后每个活块都被 smartalloc 的队列指着，LSan 会一律
// 判成 "still reachable" ⇒ 那条路必须 `--no-default-features`（见 README）。

/// **接管点**：debug 档（且带 `smartalloc` feature）的宿主堆从这里过。
#[cfg(all(feature = "smartalloc", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: smartalloc::SmartAlloc = smartalloc::SmartAlloc;

/// 收尾转储：把此刻仍在 smartalloc 账上的缓冲（"孤儿缓冲"）打出来。
///
/// 未接管时（`--no-default-features` 或 release）是 no-op。
pub fn dump_orphans() {
    #[cfg(all(feature = "smartalloc", debug_assertions))]
    smartalloc::sm_dump(true);
}

#[cfg(test)]
mod tests;

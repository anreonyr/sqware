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
//! 内核侧的对应判据是框架档的用例（`health/{pagetable,spare,stress}.rs`），两边不共享代码，
//! 只共享同一批不变量。
//!
//! # 判据分四层（见 README）
//!
//! | 层 | 通道 | 判什么 |
//! |---|---|---|
//! | ① 后端（**三选一**，见本文件接管段） | `smartalloc` / `mockalloc` / `dhat` | 孤儿块 / 分配释放**配对** / 宿主堆**用量** |
//! | ①′ 零依赖后端 | `RUSTFLAGS=-Zsanitizer=leak`（**必须不接管**） | LeakSanitizer 看宿主堆 |
//! | ② 影子账（`harness` + `tests.rs`） | 本 crate 自己 | 区间不重叠 / 指针满足对齐 / 交付区写读一致 / `block.rs:310` 的请求门必须拒 |
//! | ③ 随机序列（`tests/{pairing,heap}.rs` + `harness::plan`） | proptest | 把②的判据铺到随机分配/释放序列与随机工作负载上 |
//! | ④ 当场炸的通道（[`fault::allocator_fault`]） | 本 crate 自己 | 分配器**自身**违例（与"对象泄漏"分开归因） |
//!
//! ②③ 两条腿**不依赖后端**：`plain` / `smartalloc` / `mockalloc` / `dhat` 四档都跑同一批判据，
//! 后端只换读数口径。

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
        INFO.get()
            .expect("machine not configured (call configure first)")
    }

    pub fn hart_count() -> usize {
        *HARTS.get().unwrap_or(&4)
    }

    // **一个**线程局部槽，两个函数共用：`thread_local!` 每次展开都是一个**独立的静态**，
    // 在 `hart_id` / `hart_bind` 各写一份就等于各写各的（绑定写进 A、读取读的是 B —— 实测
    // 症状：绑到 hart 1 的线程取到的块仍归属池 0）。
    thread_local! {
        static HART: core::cell::Cell<Option<usize>> = const { core::cell::Cell::new(None) };
    }
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    /// 当前 hart（宿主上 = **当前线程**）。内核用它选 per-hart 池（`block.rs` 的
    /// `blocks[hart_id]`），宿主上"线程即 hart"，于是并发那层天然压到两条路径：
    ///
    /// * 不同线程落在**不同池**上（`hart_bind` 给确定性绑定，见下）；
    /// * 跨线程归还走 `feed`（不是本池）→ 本池下次 `pull` 时 `suck` 抽空泵 ——
    ///   `feed`/`suck` 是块池唯二的过境路径（`block.rs` 的 pump）。
    ///
    /// 单线程用例里恒 0（与"没有多线程"时的旧行为一致）：id 按**首次调用顺序**轮转分配，
    /// 主线程第一个拿到 0。
    pub fn hart_id() -> usize {
        HART.with(|c| {
            c.get().unwrap_or_else(|| {
                let n = hart_count();
                let id = if n == 0 {
                    0
                } else {
                    NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed) % n
                };
                c.set(Some(id));
                id
            })
        })
    }

    /// **宿主专用**（内核没有这个函数）：把当前线程绑到指定 hart。
    ///
    /// 并发的判据要的是"哪个线程 = 哪个池"**确定**，而不是"线程先到先得"：
    /// `pool_of(hart 1 取到的块) == 1` 这类断言在轮转分配下时对时错。
    pub fn hart_bind(id: usize) {
        HART.with(|c| c.set(Some(id)));
    }
}

/// `crate::runtime::diagnose::trace` 的宿主 shim —— `spare::init` 只问一句"每 hart 的 trace
/// 环多大"（仓容量 = 环字节 + 块头 + `DUMP_BUDGET`，再页对齐）。
///
/// 宿主上给**常量**：内核那份的真实尺寸由 `diagnose` 定，本 crate 不判它的账。它只影响仓
/// **容量**（1 MiB 级的常数），不影响 spare 的判据（预算即契约 / 拉满返 `Err` / 全归还余量还原）。
pub mod runtime {
    pub mod diagnose {
        pub mod trace {
            /// 每 hart 的 trace 环字节数（宿主常量：4 KiB/hart）。
            pub const RING_PER_HART: usize = 4096;
            /// 总环字节数（内核同名函数的宿主版）。
            pub fn ring_bytes(harts: usize) -> usize {
                RING_PER_HART * harts.max(1)
            }
        }
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
        /// 分流器（≤ 2 KiB 走 block、否则走 frame）。宿主上**不需要 init**：
        /// 它只是一个 `static` 路由器（`HYBRID_ALLOCATOR` 是 `const fn new()`），
        /// 两端的块/帧由 `harness` 装台。
        pub mod hybrid {
            include!("../../../kernel/src/memory/allocator/hybrid.rs");
        }
        /// 后备仓（崩溃路径专用仓：有序合并 free-list，一次取定、此后只在区内拉还）。
        /// 装台时它从混合分配器整块取一段仓区（`ring_bytes(harts) + HEADER + DUMP_BUDGET`），
        /// 故它是**台面的一部分**：容量读数与判据都看得见它。
        pub mod spare {
            include!("../../../kernel/src/memory/allocator/spare.rs");
        }

        /// 统计出口的宿主版：**只是计数**，不参与分配器正确性判据。
        ///
        /// 与内核 `statistics.rs` 的**写侧**同名同形（内核那份的读侧是 health 用例的判据
        /// 输入；宿主侧那批判据由影子账自己算 ⇒ 这里只留写侧）。内核源码改了写侧签名，
        /// 这里必须跟着改 —— 它是 shim，不是内核的一部分（`include!` 进来的源码**逐字未改**）。
        ///
        /// # 两条刻意的"不镜像"（都要交代，否则读数是假的）
        ///
        /// ① **类目表不建**：内核 debug/framework 档的 `install_frame_kinds` /
        ///    `install_block_kinds` 会分配逐帧、逐页的类目表（那是 §5.1 回到内核的那套
        ///    标注账）。宿主 shim 里它们是**空壳** —— 本 crate 的判据不看类目（`tag!` 在宿主上
        ///    恒等），而且"一个事实只有一份账"：在宿主上再实现一遍表尺寸，必然与内核漂移。
        ///    **代价**：本 crate 的宿主**用量**读数不含内核 debug 档那份类目表
        ///    （要看它得在内核侧量，见 README「已知边界」）。
        /// ② **读侧不镜像**：`frame_occupied` / `block_occupied` / `kinds` 那些读数在宿主上
        ///    没有读者（判据走影子账），留着只会变成第二套口径。
        pub mod statistics {
            use std::sync::atomic::{AtomicUsize, Ordering};

            // 写侧计数：只用来证明"分配器的热路径确实走到了"，不参与任何判据。
            pub static FRAME_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static FRAME_GIVE: AtomicUsize = AtomicUsize::new(0);
            pub static BLOCK_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static BLOCK_GIVE: AtomicUsize = AtomicUsize::new(0);
            pub static POOL_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static POOL_GIVE: AtomicUsize = AtomicUsize::new(0);
            pub static SPARE_TAKE: AtomicUsize = AtomicUsize::new(0);
            pub static SPARE_GIVE: AtomicUsize = AtomicUsize::new(0);
            pub static SPARE_TOTAL: AtomicUsize = AtomicUsize::new(0);

            /// 内核那份在这里装配 `STATS` 单例；宿主侧没有单例（全是原子计数）。
            pub fn init() {}

            /// 见模块头注①：宿主上**不建**逐帧类目表。
            ///
            /// # Errors
            ///
            /// 恒 `Ok`（内核那份在表分配失败时返 `OutOfMemory`）。
            pub fn install_frame_kinds(_frames: usize) -> Result<(), super::InitError> {
                Ok(())
            }

            /// 见模块头注①：宿主上**不建**逐页类目表。
            ///
            /// # Errors
            ///
            /// 恒 `Ok`（同上）。
            pub fn install_block_kinds(_base: usize, _len: usize) -> Result<(), super::InitError> {
                Ok(())
            }

            pub fn record_frame_take(_index: usize, _power: usize) {
                FRAME_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_frame_give(_index: usize) {
                FRAME_GIVE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_block_take(_addr: usize, _power: usize) {
                BLOCK_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_block_give(_addr: usize, _power: usize) {
                BLOCK_GIVE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_pool_take() {
                POOL_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_pool_give() {
                POOL_GIVE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_spare_take(_bytes: usize) {
                SPARE_TAKE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_spare_give(_bytes: usize) {
                SPARE_GIVE.fetch_add(1, Ordering::Relaxed);
            }
            pub fn record_spare_total(total: usize) {
                SPARE_TOTAL.store(total, Ordering::Relaxed);
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

// ── 宿主堆的接管：三个后端**三选一**（**debug 档**）────────────────────────────
//
// rustc 只允许一个 `#[global_allocator]`，而这三个后端的读数口径互不相同，故**三选一**；
// `run.sh` 各给一条路，`compile_error!` 把"同时开两个"挡在编译期：
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
    "宿主堆只能由一个后端接管：`smartalloc` / `mockalloc` / `dhat` **三选一**（见 README「六条路」与 run.sh）"
);

// ── 后端甲：`smartalloc`（默认）——"孤儿缓冲"转储 ──────────────────────────────
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
//    故 `.cargo/config.toml` 里钉 `RUST_TEST_THREADS = "1"`（crate 自己的使用前提；
//    另两个后端也各自要它：dhat 的读数是**进程级**的，并发用例会把别人的分配算进增量）。
//
// 与 `-Zsanitizer=leak` 互斥：接管之后每个活块都被 smartalloc 的队列指着，LSan 会一律
// 判成 "still reachable" ⇒ 那条路必须 `--no-default-features`（见 README）。
/// **接管点甲**：debug 档（且带 `smartalloc` feature）的宿主堆从这里过。
#[cfg(all(feature = "smartalloc", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: smartalloc::SmartAlloc = smartalloc::SmartAlloc;

// ── 后端乙：`mockalloc`（第一层·甲）——分配/释放**配对**判词 ─────────────────────
//
// `Mockalloc<System>` 包住系统分配器，按**线程局部**状态记每一次 alloc/free 的
// (指针, 尺寸, 对齐) 散列累加；`record_allocs(闭包)` 圈出"一幕"，收幕时看累加器归不归零
// ⇒ 泄漏 / 重复释放 / 错指针 / 错尺寸 / 错对齐 五类各自点名（`AllocError`）。
//
// 本 crate 用它**只**判一件事：内核分配器**自己**在宿主堆上的分配（记账表 / Vec / Box）
// 配不配对。它看不见台面内部（`frame`/`block` 管的是一块 16 MiB 宿主缓冲的内部划分，
// 不是全局堆）⇒ "帧发重了"那类缺陷由影子账判（`harness`）；两把尺子各管一段，别互相顶替。
//
// 为何不需要 vendored 补丁：它包的是 `System`，超 16 字节的对齐走 std 自己的
// `posix_memalign` 路径（smartalloc 那条路要补，是因为它的 C 头把基址抬了 40 字节）。
/// **接管点乙**：debug 档（且带 `mockalloc` feature）的宿主堆从这里过。
#[cfg(all(feature = "mockalloc", debug_assertions))]
#[global_allocator]
static HOST_ALLOCATOR: mockalloc::Mockalloc<std::alloc::System> =
    mockalloc::Mockalloc(std::alloc::System);

// ── 后端丙：`dhat`（第一层·乙）——宿主堆**用量**读数 ──────────────────────────
//
// `dhat::Alloc` 记每一次分配的 (块数, 字节数)，分**累计**（`total_*`）、**当下**（`curr_*`）、
// **峰值**（`max_*`）三套读数（`HeapStats::get()`）；testing 档不写 `dhat-heap.json`，
// 只供测试断言（`Profiler::builder().testing().build()`）。
//
// 它给出的正是"分配器自己占了多少宿主内存"：装台期分配多少、稳态还增长不增长、峰值被抬到哪。
// 判据在 `tests/heap.rs`。
//
// 注意：dhat 的 `Profiler` **一个进程只能建一个**（上游 `build()` 里断言旧的是 `None`），
// 故那条腿是**单用例**（`tests/heap.rs` 只有一个 `#[test]`）—— 这也正合意：三段判据
// （凭证 / 装台预算 / 随机工作负载）必须在同一幕里顺序走。
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

/// **并发台面**：线程即 hart（见下 `machine::hart_id`），判据在 `tests/mt.rs`。
pub mod concurrent;

#[cfg(test)]
mod tests;

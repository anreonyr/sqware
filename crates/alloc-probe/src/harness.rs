//! 台面与模型 —— 第一层两条腿（`mockalloc` / `dhat`）与 `tests.rs` 共用的**被压测对象**
//! 与**判据**。
//!
//! 本模块**不含任何检测后端**（不依赖 mockalloc / dhat / proptest），只做三件事：
//!
//! ① **装台**：一块 16 MiB 的宿主缓冲冒充"物理内存"，按内核 `hybrid::init` 的顺序初始化
//!    `bump` → `block` → `frame`（顺序不可换，见 `block::init` 的文档）；
//! ② **随机序列**：[`plan`] 把任意字节串翻译成 [`Op`] 序列，[`run`] 执行它 —— proptest
//!    与"把最小输入贴回语料复现"共用这一个定义，故**换随机源不改判据**；
//! ③ **影子账模型**：在册交付区间、写读模式、对齐契约、以及 `block.rs:310` 的请求门，
//!    任何一条不成立都由 [`Violation`] 当场指名。它**只**报分配器自身的问题 ——
//!    "谁没 drop"在这一层根本不在场（那正是本 crate 存在的理由）。
//!
//! 一条硬约束贯穿全文件：**[`run`] 自身零宿主分配**（状态全在栈上的固定容量数组，
//! 没有 Vec / HashMap / String / format!）。两条腿都把 `run` 放进各自的"幕"里读账，
//! 幕内只要有**任何**别的分配就会污染读数：mockalloc 会把它当成配对对象，
//! dhat 会把它算进字节增量。
//!
//! # 影子账判什么（逐条对应内核源码里的一句话）
//!
//! | 判据 | 依据 | 违反长什么样 |
//! |---|---|---|
//! | 交付区不与任何在册区间重叠 | `frame::pagemeta` 是"这页在不在手"的唯一答案（`frame.rs:18/68`） | 同一段内存被交付两次 ⇒ 两个调用方写同一块 |
//! | 交付指针满足 `layout.align()` | `Allocator` 的契约（`GlobalAlloc` 同理） | 上游 smartalloc 就栽在这条上（见 lib.rs 接管段①） |
//! | 交付区写读一致 | 交付区是调用方的，分配器**自己**的簿记不得踩进来 | 分配器把 `Link`/`Meta` 写进了已交付的块 |
//! | `block.rs:310` 的请求门必须**拒** | `power > MAX_POWER \|\| align > 1 << power` 是防御门 | 出界/超对齐的请求被接受 ⇒ 交付出来就是越界写 |
//! | 预算内的合法请求必须**成** | 台面（16 MiB）远大于在册预算（8 MiB） | 返 `Err` 只可能是 freelist 坏了或假性耗尽 |

use core::alloc::{Allocator, Layout};
use core::fmt;
use core::ptr::NonNull;
use std::sync::OnceLock;
use std::vec::Vec;

use crate::machine;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::{block, bump, frame, hybrid, spare, statistics};

/// 页大小（内核 `memory::PAGE_SIZE` 的宿主镜像）。
pub const PAGE: usize = PAGE_SIZE;

// ── 装台 ─────────────────────────────────────────────────────────────────────

/// 台面页数：16 MiB。内核那边由设备树报的 free 区决定，宿主这边我们说了算。
///
/// Miri 下缩到 1024 页（4 MiB）：16 MiB 台面在解释执行下不划算，但 4 MiB 是下限（见下），
/// 而 Miri 要看的是 **UB**（裸指针来去、交付区写读），台面大小不改变那件事。
/// 依赖台面规模的用例（容量守恒那条）在 Miri 下是 `ignore`，见 `tests.rs`。
#[cfg(not(miri))]
const ARENA_PAGES: usize = 4096;
// 1024 页（4 MiB）：**不能再小** —— 后备仓的仓区是 `ring_bytes(4) + 32 + DUMP_BUDGET(1 MiB)`
// 再按 buddy 取整（只出 2 的幂页块）⇒ 实占 **2 MiB**，`DUMP_BUDGET` 是内核常量动不得。
// 实测教训：缩到 128 页时 Miri 直接 FAILED（装台就取不到仓区）。
#[cfg(miri)]
const ARENA_PAGES: usize = 1024;
/// 冒充的 hart 数（内核按 hart 数建 per-hart 池；4 是内核 `hart_count` 的默认值）。
const HARTS: usize = 4;

/// 一块冒充"物理内存"的宿主缓冲：页对齐起点、长度页整数倍。
pub struct Arena {
    /// 缓冲本体（**必须持有**：分配器交付的指针都指进这块内存）。
    v: Vec<u8>,
    base: usize,
    pages: usize,
}

impl Arena {
    fn new(pages: usize) -> Self {
        let size = pages * PAGE;
        // 多要一页，取其中页对齐的起点：保证 [base, base + size) 全在缓冲内。
        let v: Vec<u8> = vec![0; size + PAGE];
        let raw = v.as_ptr() as usize;
        let base = raw.next_multiple_of(PAGE);
        Self { v, base, pages }
    }

    /// 台面基址（喂给 `machine::configure` 当"物理内存起点"）。
    pub fn base(&self) -> usize {
        self.base
    }

    /// 台面页数。
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// 台面字节数。
    pub fn size(&self) -> usize {
        self.pages * PAGE
    }
}

static ARENA: OnceLock<Arena> = OnceLock::new();
static INIT: OnceLock<()> = OnceLock::new();
#[cfg(feature = "mockalloc")]
static INIT_ALLOCS: OnceLock<mockalloc::AllocInfo> = OnceLock::new();

/// 台面（16 MiB 宿主缓冲）。**测试自己的台面，不属于分配器的账**：故 dhat 那条腿
/// 在量"装台预算"时，先把台面拿到手再开窗（否则 16 MiB 会盖掉分配器那几 KB 的读数）。
pub fn arena() -> &'static Arena {
    ARENA.get_or_init(|| Arena::new(ARENA_PAGES))
}

/// 装台：`configure` + `bump` → `block` → `frame`，**恰一次**（`OnceLock` 幂等）。
///
/// `mockalloc` 档下，**这整段在记录幕里跑**：装台是分配器唯一会做宿主分配的地方
/// （记账表 / Vec / Box，见本文件 `INIT_*` 常量），故它的配对账是那条腿的主要判据。
///
/// 注意：**不要**在任何别的记录幕里调它 —— `mockalloc` 的 `record_allocs` 不允许嵌套
/// （嵌套会 `assert!("Mockalloc already recording")`）。`boot()` 在每条用例开头调，
/// 幕只在这里面开。
pub fn init() {
    INIT.get_or_init(|| {
        let a = arena();
        let run = || {
            machine::configure(a.base(), a.size(), HARTS);
            bump::init().expect("bump::init");
            // **block 必须在 frame 之前**（`block.rs` 的 `init` 文档：池的页要向 frame 借，
            // 而簿记表要覆盖 free 区；顺序反了帧链的链头就是毒化内存 —— 实测过）。
            block::init().expect("block::init");
            frame::init().expect("frame::init");
            // 后备仓：内核 `allocator::init()` 的最后一步（从混合分配器整块取仓区 ⇒ 必须排在
            // 块/帧之后）。`hybrid` 只是 `static` 路由器，不需要 init。
            spare::init().expect("spare::init");
            statistics::record_spare_total(spare::spare().total_bytes());
        };
        #[cfg(feature = "mockalloc")]
        {
            let info = mockalloc::record_allocs(run);
            let _ = INIT_ALLOCS.set(info);
        }
        #[cfg(not(feature = "mockalloc"))]
        run();
    });
}

/// 台面 + 装台（用例的一行入口）。
pub fn boot() -> &'static Arena {
    let a = arena();
    init();
    a
}

/// `mockalloc` 档：装台幕的账（`init()` 之后才拿得到）。仅该档存在 —— 调用方本来就
/// 只可能在 `tests/pairing.rs` 里（那里整文件 `#![cfg(feature = "mockalloc")]`）。
#[cfg(feature = "mockalloc")]
pub fn init_allocs() -> Option<&'static mockalloc::AllocInfo> {
    INIT_ALLOCS.get()
}

// ── 装台幕的**逐笔账**（两个后端读同一次 init，按定义必须一致）──────────────────
//
// 这组常量是 `mockalloc`（`num_allocs` / `mem_leaked`）与 `dhat`（`total_blocks` /
// `curr_bytes` 增量）**共用的同一本账**：两个后端从不同口径读同一段代码，常数钉在一处，
// 谁漂了都看得见（口径为何按定义相等，见 `tests/heap.rs` 的头注）。
//
// 怎么重标定：`./run.sh mockalloc -- --nocapture` 与 `./run.sh dhat -- --nocapture`，
// 两处都会把实测值打在 `[init-ledger]` 行上。**先看是不是分配器多留了东西**：
// 笔数变了 = 有人偷偷加了一笔宿主分配（那就是本层要抓的"未配对"）；字节数变了但笔数没变
// = 某张表的尺寸变了（合法重构，改预算即可）。
//
// 实测（2025-09，x86-64，debug 档，台面 4096 页）：9 笔 / 11308 B，**9 笔全部未配对**
// —— 装台期连一笔"取了又还"的临时分配都没有。

/// 装台期的**分配事件数**（mockalloc 的 `num_allocs` / dhat 的 `total_blocks` 增量）。
///
/// 实测 **9**，而且这 9 笔**全部**是终身表（下面那张清单）：装台期一笔临时宿主分配都没有
/// （`Vec<BlockInner>` 的增长与 `into_boxed_slice` 恰好共用同一次分配，其结果就是那张
/// `'static` 表；`Vec::try_reserve` 的容量一次到位，没有二次增长）。
pub const INIT_BLOCKS: u64 = 10;

/// 装台期**故意不还**的笔数 = 分配器的终身表（`'static`，进程活多久它活多久）：
///
/// | # | 笔 | 出处 |
/// |---|---|---|
/// | 1 | `Box<FrameAllocator>` | `frame.rs:558`（`SpinLock` 非 `Send` ⇒ 存 `&'static`） |
/// | 2 | `freelist` 的 Vec 缓冲 | `frame.rs:165`（`max_power` 条 `Option<NonNull<Link>>`） |
/// | 3 | `pagemeta` 的 Vec 缓冲 | `frame.rs:169`（free 区每页一条 `Option<Meta>`） |
/// | 4 | `Box<Tally>` | `block.rs:283`（簿记表本身） |
/// | 5 | `Box<[BlockInner]>` | `block.rs:296`（per-hart 池集合） |
/// | 6–9 | 每池 `freepool` 的 Vec 缓冲 ×4 | `block.rs:590`（`Pool::init` 的 `try_reserve`） |
///
/// 这 9 笔是**设计**（表要活到进程结束），不是缺陷：判据是"**除它们之外**一笔都不能不配对"。
/// 笔数变化 ⇒ 有人加了一笔没还的分配 ⇒ 这条用例当场点名。
pub const INIT_LIFETIME_LEAKS: u64 = 10;

/// 装台期宿主分配**总量**的上界（含配对的临时）：实测 11308 B，预算取 ~23 倍余量。
pub const INIT_BYTES_BUDGET: u64 = 256 * 1024;

/// 装台期**常驻**（不还的那部分）字节上界：实测 11308 B（与总量相等 —— 一笔都没还），
/// 预算取 ~5.8 倍余量。
///
/// 它同时是"分配器的宿主簿记有多胖"这条读数的判据 —— 尺寸随台面变大而变大
/// （`pagemeta` 每页 2 B），故这是**上界**而不是等式。
pub const INIT_LIVE_BUDGET: u64 = 64 * 1024;

// ── 请求：两张表 + 契约 ───────────────────────────────────────────────────────

/// 请求落到哪个后端。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// 帧（buddy，页的整数倍）。
    Frame,
    /// 块（segregated free list，≤ 半页）。
    Block,
    /// 混合分流器（≤ 2 KiB 走块、否则走帧）—— 判的是**路由**本身。
    Hybrid,
    /// 后备仓（崩溃路径专用仓：有序合并 free-list，一次取定、此后只在区内拉还）。
    Spare,
}

impl Backend {
    /// 内核那边的入口（`frame::allocator()` / `block::allocator()`，都是 `&'static dyn Allocator`）。
    pub fn allocator(self) -> &'static dyn Allocator {
        match self {
            Backend::Frame => frame::allocator(),
            Backend::Block => block::allocator(),
            Backend::Hybrid => hybrid::allocator(),
            Backend::Spare => spare::allocator(),
        }
    }

    /// 判词里用的名字。
    pub fn name(self) -> &'static str {
        match self {
            Backend::Frame => "frame",
            Backend::Block => "block",
            Backend::Hybrid => "hybrid",
            Backend::Spare => "spare",
        }
    }
}

/// `block` 的请求表 `(字节数, 对齐)`；[`Op::Take`] 的 `req` 是它的下标。
///
/// 前 14 条**合法**（size class ≤ 半页 2048，对齐不超过该 size class 的尺寸）；
/// 后 4 条按 `block.rs:310` 的守护门必须被**拒**：越界/超对齐的请求一旦被交付，
/// 调用方拿到的就是一段越界的块。
const BLOCK_REQS: [(usize, usize); 18] = [
    (1, 8),
    (3, 8),
    (6, 8),
    (8, 8),
    (16, 8),
    (24, 8),
    (64, 8),
    (100, 8),
    (256, 16),
    (512, 8),
    (1000, 8),
    (1024, 16),
    (2000, 8),
    (2048, 8),
    // ↓↓ 越界 / 超对齐：契约要求 `Err`
    (2049, 8),  // power 12 > MAX_POWER 11
    (4096, 8),  // power 12
    (8192, 64), // power 13
    (16, 64),   // align 64 > 1 << power(4)
];

/// `frame` 的请求表（页数）。`frame` 没有上界门，靠 `split_block` 拿不到块时返 `Err`；
/// 台面 16 MiB ⇒ 这 8 条都在台面之内，**必须成功**（拿到 `Err` 就是 freelist 坏了或假性耗尽）。
const FRAME_PAGES: [usize; 8] = [1, 1, 2, 3, 4, 8, 16, 32];

/// `hybrid` 的请求表 `(字节数, 对齐)`：前四条走**块**（≤ 2 KiB，对齐不超 size class），
/// 后四条走**帧**（> 2 KiB，页对齐）。两侧都必须成功 —— 这正是"分流点的两侧都通"这条判据。
const HYBRID_REQS: [(usize, usize); 8] = [
    (8, 8),
    (64, 8),
    (512, 16),
    (2048, 8),
    (2049, 8),
    (4096, 8),
    (8192, 8),
    (16384, 8),
];

/// `spare` 的请求表 `(字节数, 对齐)`：载荷级请求（1 B…4 KiB），档位都在仓区容量之内。
const SPARE_REQS: [(usize, usize); 8] = [
    (1, 1), (16, 8), (32, 16), (64, 8), (100, 16), (256, 8), (1024, 16), (4096, 8),
];

/// 请求的**契约**：这条请求该成功还是该被拒。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Want {
    /// 合法请求：预算内**必须**成功。
    Ok,
    /// 越界/超对齐请求：**必须**返 `Err`（`block.rs:310` 的守护门）。
    Err,
}

/// `block` 的契约（**镜像** `block.rs:304-312` 那三行判据，不是另立一套）：
/// `power = max(size, 1 << MIN_POWER).next_power_of_two().ilog2()`，合法 ⇔
/// `power <= MAX_POWER`（= `(PAGE_SIZE / 2).ilog2()` = 11）且 `align <= 1 << power`。
fn block_want(size: usize, align: usize) -> Want {
    const MIN_POWER: usize = 3;
    const MAX_POWER: usize = 11; // (4096 / 2).ilog2()
    let power = size.max(1 << MIN_POWER).next_power_of_two().ilog2() as usize;
    if power > MAX_POWER || align > (1usize << power) {
        Want::Err
    } else {
        Want::Ok
    }
}

/// 一条请求 → `(layout, 契约)`。
///
/// `pub(crate)`：并发那层（`src/concurrent.rs`）用**同一张请求表**与**同一个契约门**，
/// 两层才是同一批判据（差别只在影子账是栈上的、还是原子登记表）。
pub(crate) fn request(backend: Backend, req: u8) -> (Layout, Want) {
    match backend {
        Backend::Block => {
            let (size, align) = BLOCK_REQS[req as usize % BLOCK_REQS.len()];
            (
                Layout::from_size_align(size, align).expect("请求表是静态的，必然合法"),
                block_want(size, align),
            )
        }
        Backend::Frame => {
            let pages = FRAME_PAGES[req as usize % FRAME_PAGES.len()];
            (
                Layout::from_size_align(pages * PAGE, PAGE).expect("请求表是静态的，必然合法"),
                Want::Ok,
            )
        }
        Backend::Hybrid => {
            // 分流器**不设门**：它只按长度路由，契约在两端各自那层。
            // （第一版这里把**块**的契约门套到了 hybrid 上，于是 2049 B 的请求被判"该拒"——
            //  而 hybrid 只会把它转给帧，帧本来就收。判据的契约必须跟着**被测后端**走。）
            let (size, align) = HYBRID_REQS[req as usize % HYBRID_REQS.len()];
            (
                Layout::from_size_align(size, align).expect("请求表是静态的，必然合法"),
                Want::Ok,
            )
        }
        Backend::Spare => {
            let (size, align) = SPARE_REQS[req as usize % SPARE_REQS.len()];
            (
                Layout::from_size_align(size, align).expect("请求表是静态的，必然合法"),
                // `spare.rs`：`align > MAX_ALIGN(16)` ⇒ Err（不 panic）—— 表里只放合法档位。
                Want::Ok,
            )
        }
    }
}

// ── 序列 ─────────────────────────────────────────────────────────────────────

/// 随机序列里的一条动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// 取一块：`req` 是 `BLOCK_REQS` / `FRAME_PAGES` 的下标。
    Take { backend: Backend, req: u8 },
    /// 还在册的第 `pick` 块（`pick % 在册数`）——**同一对 (ptr, layout) 原样归还**。
    Give { pick: u8 },
    /// 把在册的全部归还（逆序 ⇒ 逼出伙伴合并）。
    GiveAll,
}

/// 字节串 → [`Op`] 序列：**每两个字节一条**（奇数长度丢弃尾字节）。
///
/// 这是"随机序列"的**唯一**入口：proptest 生成的是字节，序列的定义在这里。于是
/// ① 换随机源不改判据；② proptest 打印的最小输入可以直接贴回语料复现（见 README）。
///
/// 权重：取块 1/2（block 1/4、frame 1/4）、还单块 3/8、还全部 1/8 —— 取还大致持平，
/// 在册块数围绕一个不高的水位抖动（这正是伙伴合并/池泵最容易出错的地方）。
pub fn plan(bytes: &[u8]) -> Vec<Op> {
    bytes
        .chunks_exact(2)
        .map(|c| {
            let (k, a) = (c[0], c[1]);
            match k % 8 {
                0 => Op::Take {
                    backend: Backend::Block,
                    req: a,
                },
                1 => Op::Take {
                    backend: Backend::Hybrid,
                    req: a,
                },
                2 => Op::Take {
                    backend: Backend::Frame,
                    req: a,
                },
                3 => Op::Take {
                    backend: Backend::Spare,
                    req: a,
                },
                4 | 5 | 6 => Op::Give { pick: a },
                _ => Op::GiveAll,
            }
        })
        .collect()
}

/// 确定性语料的一条：`seed` 起算的最简 LCG（Numerical Recipes 常数）出 `len` 字节。
///
/// 它让**零后端档**（`plain`）也跑随机序列 —— proptest 只在两条腿上探索，
/// 但判据本身不该依赖"有没有装 proptest"。
pub fn lcg_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut s = seed
        .wrapping_mul(2862933555777941757)
        .wrapping_add(3037000493);
    (0..len)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as u8
        })
        .collect()
}

// ── 执行 + 影子账 ────────────────────────────────────────────────────────────

/// 影子账容量：一幕里最多同时在册的块数（满了 `Take` 不落到分配器上，记 `skipped`）。
const MAX_LIVE: usize = 48;

/// 在册**字节**预算（[`Limits::STRICT`] 用）：远小于台面 16 MiB ⇒"合法请求必成功"
/// 这条判据才成立 —— 真耗尽不该发生，于是返 `Err` 只可能是分配器自己的病。
const HOLD_BUDGET: usize = 8 * 1024 * 1024;

/// 一幕的档位。
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// 在册字节上限：超过则 `Take` 跳过（不落到分配器上）。
    pub budget: usize,
    /// 台面满时的 `Err`：`true` = 记账（本幕就是来撞天花板的），`false` = 分配器缺陷。
    pub tolerate_reject: bool,
}

impl Limits {
    /// **常规幕**：预算内 ⇒ 合法请求必成功，`Err` 只可能来自分配器自身。
    pub const STRICT: Limits = Limits {
        budget: HOLD_BUDGET,
        tolerate_reject: false,
    };
    /// **撞天花板幕**：把台面取到 `Err` 为止才算数（`tests.rs` 的耗尽—恢复用例）。
    pub const FILL: Limits = Limits {
        budget: usize::MAX,
        tolerate_reject: true,
    };
}

/// 一幕跑完的计数（判据之外的读数：写进失败消息，也用来证明序列真的落地了）。
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Report {
    /// 计划里的条数。
    pub ops: usize,
    /// 因影子账满/超预算而没落到分配器上的条数。
    pub skipped: usize,
    /// 成功交付的块数。
    pub taken: usize,
    /// 分配器返 `Err` 的请求数（含契约要求的那几条）。
    pub rejected: usize,
    /// 归还的块数。
    pub given: usize,
    /// 在册峰值块数。
    pub peak_live: usize,
    /// 在册峰值字节数。
    pub peak_bytes: usize,
}

/// 分配器**自身**的违例 —— 与"谁没 drop"无关，故在这一层当场指名。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Violation {
    /// 交付区间与在册块重叠：同一段内存被交付了两次。
    Overlap {
        backend: Backend,
        new: (usize, usize),
        old: (usize, usize),
    },
    /// 交付指针不满足 `layout.align()`（`Allocator` 的契约）。
    Misaligned {
        backend: Backend,
        addr: usize,
        align: usize,
    },
    /// 交付区被写坏：分配器自己的簿记（`Link` / `Meta`）踩进了已交付的块。
    Pattern {
        backend: Backend,
        addr: usize,
        at: usize,
        want: u8,
        got: u8,
    },
    /// 契约要求拒绝的请求被**接受**了（`block.rs:310` 的守护门失灵）。
    Accepted {
        backend: Backend,
        size: usize,
        align: usize,
    },
    /// 预算内的合法请求被**拒**：freelist 损坏或假性耗尽（不是"台面真没了"）。
    Rejected {
        backend: Backend,
        size: usize,
        align: usize,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Violation::Overlap {
                backend,
                new: (a, n),
                old: (b, m),
            } => write!(
                f,
                "{}: 交付区间重叠 —— 新 {a:#x}+{n} 与在册 {b:#x}+{m}（同一段内存被交付两次）",
                backend.name()
            ),
            Violation::Misaligned {
                backend,
                addr,
                align,
            } => write!(
                f,
                "{}: 交付指针 {addr:#x} 不满足 layout.align()={align}",
                backend.name()
            ),
            Violation::Pattern {
                backend,
                addr,
                at,
                want,
                got,
            } => write!(
                f,
                "{}: 交付区被写坏 —— {addr:#x}+{at} 期望 {want:#x} 实得 {got:#x}\
                 （分配器的簿记踩进了已交付的块，或同一区间被交付两次）",
                backend.name()
            ),
            Violation::Accepted {
                backend,
                size,
                align,
            } => write!(
                f,
                "{}: 越界/超对齐请求被接受 —— size={size} align={align} 该被拒（block.rs:310 的守护门）",
                backend.name()
            ),
            Violation::Rejected {
                backend,
                size,
                align,
            } => write!(
                f,
                "{}: 预算内的合法请求被拒 —— size={size} align={align}\
                 （freelist 损坏或假性耗尽；台面只用了不到一半）",
                backend.name()
            ),
        }
    }
}

/// 在册的一块（影子账的一条）。
#[derive(Clone, Copy)]
struct Live {
    backend: Backend,
    ptr: NonNull<u8>,
    addr: usize,
    len: usize,
    layout: Layout,
    tag: u8,
}

/// 跑一幕（[`Limits::STRICT`]）。
pub fn run(ops: &[Op]) -> Result<Report, Violation> {
    run_with(ops, Limits::STRICT)
}

/// 跑一幕：每一步都对照影子账复核（重叠 / 对齐 / 写读一致 / 契约门 / 预算内必成）。
///
/// **零宿主分配**：全部状态在栈上（`Live` 的固定容量数组 + 几个 `usize`），故调用方
/// 可以把它放进 mockalloc / dhat 的幕里读账，而读数不被本函数污染。
///
/// 收尾对账：这一幕在册的都还回去（用例不留东西 ⇒ 台面回到本幕之前的水位）。
pub fn run_with(ops: &[Op], limits: Limits) -> Result<Report, Violation> {
    let mut live: [Option<Live>; MAX_LIVE] = [None; MAX_LIVE];
    let mut n = 0usize;
    let mut held = 0usize;
    let mut seq: u8 = 0;
    let mut r = Report::default();

    for op in ops {
        r.ops += 1;
        match *op {
            Op::Take { backend, req } => {
                let (layout, want) = request(backend, req);
                if n == MAX_LIVE || held.saturating_add(layout.size()) > limits.budget {
                    // 影子账满 / 超预算：本条不落到分配器上（诚实记账，不假装跑了）。
                    r.skipped += 1;
                    continue;
                }
                match backend.allocator().allocate(layout) {
                    Err(_) if want == Want::Err => r.rejected += 1,
                    Err(_) if limits.tolerate_reject => r.rejected += 1,
                    Err(_) => {
                        return Err(Violation::Rejected {
                            backend,
                            size: layout.size(),
                            align: layout.align(),
                        });
                    }
                    Ok(buf) => {
                        let ptr = buf.cast::<u8>();
                        let len = buf.len();
                        let addr = ptr.as_ptr() as usize;
                        if want == Want::Err {
                            // 交付了就先还回去（台面不能漏），再报这条缺陷。
                            // SAFETY: 本指针是本步刚拿到的，layout 原样带回。
                            unsafe { backend.allocator().deallocate(ptr, layout) };
                            return Err(Violation::Accepted {
                                backend,
                                size: layout.size(),
                                align: layout.align(),
                            });
                        }
                        if addr % layout.align() != 0 {
                            return Err(Violation::Misaligned {
                                backend,
                                addr,
                                align: layout.align(),
                            });
                        }
                        for k in 0..n {
                            let l = live[k].expect("影子账：在册段必为 Some");
                            if addr < l.addr + l.len && l.addr < addr + len {
                                return Err(Violation::Overlap {
                                    backend,
                                    new: (addr, len),
                                    old: (l.addr, l.len),
                                });
                            }
                        }
                        // 交付区写满模式字节：归还时读回，任何"簿记踩进交付区"或
                        // "同一段被交付两次"都会在归还那一刻现行。
                        let tag = seq.wrapping_mul(37).wrapping_add(11);
                        // SAFETY: `len` 是分配器声明的交付长度，本指针本步刚拿到。
                        unsafe { core::ptr::write_bytes(ptr.as_ptr(), tag, len) };
                        live[n] = Some(Live {
                            backend,
                            ptr,
                            addr,
                            len,
                            layout,
                            tag,
                        });
                        n += 1;
                        held += len;
                        seq = seq.wrapping_add(1);
                        r.taken += 1;
                        r.peak_live = r.peak_live.max(n);
                        r.peak_bytes = r.peak_bytes.max(held);
                    }
                }
            }
            Op::Give { pick } => {
                if n == 0 {
                    r.skipped += 1;
                    continue;
                }
                give(&mut live, pick as usize % n, &mut n, &mut held)?;
                r.given += 1;
            }
            Op::GiveAll => {
                // 逆序归还：`give` 是 swap_remove，降序恰好等价于逐个弹出。
                for k in (0..n).rev() {
                    give(&mut live, k, &mut n, &mut held)?;
                    r.given += 1;
                }
            }
        }
    }

    for k in (0..n).rev() {
        give(&mut live, k, &mut n, &mut held)?;
        r.given += 1;
    }
    Ok(r)
}

/// 归还一块：先读回模式字节，再按**原样的** `(ptr, layout)` 归还。
fn give(
    live: &mut [Option<Live>; MAX_LIVE],
    k: usize,
    n: &mut usize,
    held: &mut usize,
) -> Result<(), Violation> {
    let l = live[k].expect("影子账：在册下标必在界内");
    for i in 0..l.len {
        // SAFETY: 该区间是本幕从分配器拿到的交付区，尚未归还。
        let got = unsafe { *l.ptr.as_ptr().add(i) };
        if got != l.tag {
            return Err(Violation::Pattern {
                backend: l.backend,
                addr: l.addr,
                at: i,
                want: l.tag,
                got,
            });
        }
    }
    // SAFETY: 同一次 allocate 的产物，layout 原样带回。
    unsafe { l.backend.allocator().deallocate(l.ptr, l.layout) };
    // swap_remove：末尾那块填进空位（顺序会变，但"在册集合"不变）。
    live[k] = live[*n - 1];
    live[*n - 1] = None;
    *n -= 1;
    *held -= l.len;
    Ok(())
}

// ── 容量探针（不走影子账：一条一块取到 `Err` 为止）────────────────────────────
//
// 影子账的容量（48 块）故意压得很低（每步要扫在册区间），故"取尽台面"这件事得单开一条
// 通道：它不查重叠，只问两件影子账问不了的事 —— **容量守恒**（取尽—还尽—再取尽，
// 页数必须一样）与**合并没有漏**（还尽之后必须还能拿到大块）。判据在 `tests.rs`。

/// 把 frame 以 `pages` 页一块取到 `Err` 为止，产物推进 `out`；返回取到的块数。
///
/// 调用方负责归还（[`drain`]）——本函数**不**做收尾对账，这正是它的用处：把"台面满"
/// 这个状态交给调用方，好让它接着验"满了之后会怎样"。
pub fn fill_frames(pages: usize, out: &mut Vec<(NonNull<u8>, Layout)>) -> usize {
    let layout = Layout::from_size_align(pages * PAGE, PAGE).expect("页数合法");
    let a = frame::allocator();
    let mut n = 0;
    while let Ok(buf) = a.allocate(layout) {
        out.push((buf.cast::<u8>(), layout));
        n += 1;
    }
    n
}

/// 归还 [`fill_frames`] 的产物（**原样** `(ptr, layout)`，逆序）。
pub fn drain(blocks: &mut Vec<(NonNull<u8>, Layout)>) {
    let a = frame::allocator();
    while let Some((p, l)) = blocks.pop() {
        // SAFETY: 同一次 allocate 的产物。
        unsafe { a.deallocate(p, l) };
    }
}

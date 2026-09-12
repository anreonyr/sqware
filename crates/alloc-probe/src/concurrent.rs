//! 并发台面 —— **线程即 hart**，把内核分配器放到真并发下压。
//!
//! 单线程那层（`harness`）的影子账在**栈上**：每步扫自己的在册区间，零宿主分配，判据干净。
//! 并发这层要判的是另三件事，栈上账本一件都答不了：
//!
//! | 判据 | 通道 | 依据 |
//! |---|---|---|
//! | 交付区**跨线程**不重叠 / 不被簿记踩 | 全局**原子登记表**（占位 + 扫描）+ 交付区模式写读 | 交付区是共享内存：同一段被交付两次 ⇒ 两个线程往同一段写不同 tag ⇒ 归还时必有一方读回不符 |
//! | 取还**守恒** | `statistics` 的原子计数（块级 / 帧级 / 池级） | 块级**严格守恒**；帧级/池级的差值只能是池的**迟滞**（每 size class 1 页，`block.rs` 头注） |
//! | 在途块**全部归还** | `LIVE` 原子计数 | 一趟并发跑完必须归零 |
//! | 跨 hart 归还真的**回了本池** | `block::heap().own(pa)`（池归属读数） | `home != me` 时走 `feed`；本池下次 `pull` 时 `suck` 抽回（`block.rs` 的泵是唯一回路） |
//!
//! # 两层为什么各有一份 `run` 循环
//!
//! `harness::run` 与 [`run_plan`] 的 op 循环看着像，但**账本不同**：前者是栈上的数组
//! （要零宿主分配，好让 dhat/mockalloc 在幕内读账），后者是静态上的原子表（要跨线程可见）。
//! 抽一个 trait 出来会让"零宿主分配"这条约束变成运行时才知道的事，故宁可各写一份 ——
//! 但**请求表与契约门是同一个**（`harness::request` / `harness::Want`），两层的判据才是同一批。
//!
//! # 登记表与模式写读：谁在前线
//!
//! 登记表**一把锁串行**（见下面 `Registry` 的头注）：给出逐个区间的精确重叠判定，读不到
//! "半格"。**交付区模式写读**是另一条独立通道：交付区是共享内存，两个线程往里写不同的
//! tag，归还时读回必有一方不符 —— 它不依赖登记表，两条互为交叉验证。
//!
//! （第一版登记表是"原子占位 + CAS 发布"，被这两条判据当场抓住**自己造假缺陷**：撕裂读
//! 会把 A 的地址配上 B 的长度，算出根本不存在的重叠。指纹与改法记在 `Registry` 头注里 ——
//! 判据先要自己不编缺陷，再谈能不能抓到缺陷。）

use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;
use std::vec::Vec;

use crate::harness::{self, Backend, Op, Report, Violation, Want};
use crate::machine;
use crate::memory::allocator::{block, statistics};

/// 登记表容量：一趟并发里最多同时在册的块数。
///
/// 上限由 `concurrent_churn` 的断言守住：`threads × MAX_LIVE ≤ SLOTS`（8 × 24 = 192 < 256）。
const SLOTS: usize = 256;
/// 每线程在册块上限（每步要扫自己的在册区间 + 一格登记表）。
const MAX_LIVE: usize = 24;
/// 允许的最大线程数（受登记表容量约束）。
pub const MAX_THREADS: usize = 8;

/// 交付区模式字节的生成式：`tag = seq × 37 + 11`（与单线程层同式，便于对照）。
const TAG_MUL: u8 = 37;
const TAG_ADD: u8 = 11;

// ── 全局登记表（一把锁串行）────────────────────────────────────────────────
//
// **为什么不是原子占位**：第一版是"每格 4 个原子 + CAS 发布"，被这套判据当场抓住自己
// 造**假缺陷**，指纹很干净：报出的"2000 字节的块起始于 `0x…d038`"在块池里**不可能**
// （2048 类只能落在类对齐地址上）。成因是撕裂读 —— 一个线程写字段写到一半被别人抢走那格，
// 读者就把 **A 的地址配上了 B 的长度**，于是算出一个根本不存在的重叠区间。
//
// 换成一把 `Mutex` 之后：**只锁我们的账**，分配器内部的并发一点没少（帧的 `SpinLock`、
// 池/泵/簿记表的锁照旧在真竞争下），而登记表不再可能自己造缺陷。判据宁可慢一点，
// 也不能自己编缺陷出来。
#[derive(Clone, Copy)]
struct Entry {
    addr: usize,
    len: usize,
    tag: u8,
}

struct Registry {
    slots: [Option<Entry>; SLOTS],
}

/// 锁中毒也照用（与 shim 的 `SpinLock` 同一处理）：判据要的是"把缺陷报出来"，
/// 不是"被 panic 连累到读不了账"。
fn lock_registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

static REGISTRY: std::sync::Mutex<Registry> = std::sync::Mutex::new(Registry {
    slots: [None; SLOTS],
});
/// 在册块数（原子计数只是**读数**；真正的账在上面的锁里）：一趟并发跑完必须归零。
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// 占一格。表满属于调用方违约（`threads × MAX_LIVE ≤ SLOTS` 是硬不变量），故直接炸。
fn registry_claim(addr: usize, len: usize, tag: u8) -> usize {
    let mut g = lock_registry();
    for (i, s) in g.slots.iter_mut().enumerate() {
        if s.is_none() {
            *s = Some(Entry { addr, len, tag });
            return i;
        }
    }
    panic!("登记表满：threads × MAX_LIVE ≤ SLOTS 是调用方必须守住的不变量");
}

fn registry_release(slot: usize) {
    let mut g = lock_registry();
    g.slots[slot] = None;
}

/// 有没有**别的**在册块与本段重叠。整张表在同一把锁下读 ⇒ 不会读到"半格"。
fn registry_overlap(me: usize, addr: usize, len: usize) -> Option<(usize, usize)> {
    let g = lock_registry();
    for (i, s) in g.slots.iter().enumerate() {
        if i == me {
            continue;
        }
        if let Some(e) = s {
            if addr < e.addr + e.len && e.addr < addr + len {
                return Some((e.addr, e.len));
            }
        }
    }
    None
}

/// 此刻在册的块数（并发判据的读数之一）。
pub fn registry_live() -> usize {
    LIVE.load(Ordering::Acquire)
}

// ── `statistics` 的写侧计数（原子账）────────────────────────────────────────

/// 分配器写侧计数的快照。判据看**差值**：块级严格守恒，帧级/池级只差迟滞。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub frame_take: usize,
    pub frame_give: usize,
    pub block_take: usize,
    pub block_give: usize,
    pub pool_take: usize,
    pub pool_give: usize,
}

impl Counters {
    /// 本快照相对 `before` 的增量。
    pub fn delta(self, before: Counters) -> Counters {
        Counters {
            frame_take: self.frame_take - before.frame_take,
            frame_give: self.frame_give - before.frame_give,
            block_take: self.block_take - before.block_take,
            block_give: self.block_give - before.block_give,
            pool_take: self.pool_take - before.pool_take,
            pool_give: self.pool_give - before.pool_give,
        }
    }
}

/// 读一次写侧计数（原子，`Relaxed`：只在栅栏之后取，见各用例的用法）。
pub fn counters() -> Counters {
    Counters {
        frame_take: statistics::FRAME_TAKE.load(Ordering::Relaxed),
        frame_give: statistics::FRAME_GIVE.load(Ordering::Relaxed),
        block_take: statistics::BLOCK_TAKE.load(Ordering::Relaxed),
        block_give: statistics::BLOCK_GIVE.load(Ordering::Relaxed),
        pool_take: statistics::POOL_TAKE.load(Ordering::Relaxed),
        pool_give: statistics::POOL_GIVE.load(Ordering::Relaxed),
    }
}

/// 池的**迟滞**：每个 size class（`MIN_POWER..=MAX_POWER` = 3..=11，共 9 档）至多留
/// 1 个空闲页不还（`block.rs` 头注的 arena 迟滞）—— **每池**一份。
pub const SPARE_PAGES_PER_POOL: usize = 9;

/// 帧级/池级差值的上界：**每池**迟滞 × 池数（池按 hart 数建，见 `block.rs::init`）。
pub fn spare_pages_max() -> usize {
    machine::hart_count().max(1) * SPARE_PAGES_PER_POOL
}

// ── 并发计划的执行 ───────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Live {
    backend: Backend,
    ptr: NonNull<u8>,
    addr: usize,
    len: usize,
    slot: usize,
    layout: core::alloc::Layout,
    tag: u8,
}

/// 一趟并发执行的账本（栈上：工作线程里不放宿主分配）。
struct Run {
    live: [Option<Live>; MAX_LIVE],
    n: usize,
    held: usize,
    seq: u8,
}

impl Run {
    fn new() -> Self {
        Self {
            live: [None; MAX_LIVE],
            n: 0,
            held: 0,
            seq: 0,
        }
    }

    /// 跑一份计划。**任何**出口都先把在册的还回去（登记表不能带着脏格留给下一条用例）。
    fn go(&mut self, ops: &[Op], budget: usize) -> Result<Report, Violation> {
        match self.run(ops, budget) {
            // `run` 内部已做收尾对账（`given` 计满），报告直接可用。
            Ok(r) => Ok(r),
            // 出错：把在册的还回去再上抛（失败路径也把台面还原）。
            Err(v) => {
                self.cleanup();
                Err(v)
            }
        }
    }

    fn run(&mut self, ops: &[Op], budget: usize) -> Result<Report, Violation> {
        let mut r = Report::default();
        for op in ops {
            r.ops += 1;
            match *op {
                Op::Take { backend, req } => {
                    let (layout, want) = harness::request(backend, req);
                    if self.n == MAX_LIVE || self.held.saturating_add(layout.size()) > budget {
                        r.skipped += 1;
                        continue;
                    }
                    match backend.allocator().allocate(layout) {
                        Err(_) if want == Want::Err => r.rejected += 1,
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
                                // 交付了就先还回去，再报"接受了非法请求"。
                                // SAFETY: 本指针本步刚拿到，layout 原样带回。
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
                            let tag = self.seq.wrapping_mul(TAG_MUL).wrapping_add(TAG_ADD);
                            let slot = registry_claim(addr, len, tag);
                            if let Some((a, l)) = registry_overlap(slot, addr, len) {
                                registry_release(slot);
                                // SAFETY: 同上。
                                unsafe { backend.allocator().deallocate(ptr, layout) };
                                return Err(Violation::Overlap {
                                    backend,
                                    new: (addr, len),
                                    old: (a, l),
                                });
                            }
                            LIVE.fetch_add(1, Ordering::Relaxed);
                            // SAFETY: `len` 是分配器声明的交付长度。
                            unsafe { core::ptr::write_bytes(ptr.as_ptr(), tag, len) };
                            self.live[self.n] = Some(Live {
                                backend,
                                ptr,
                                addr,
                                len,
                                slot,
                                layout,
                                tag,
                            });
                            self.n += 1;
                            self.held += len;
                            self.seq = self.seq.wrapping_add(1);
                            r.taken += 1;
                            r.peak_live = r.peak_live.max(self.n);
                            r.peak_bytes = r.peak_bytes.max(self.held);
                        }
                    }
                }
                Op::Give { pick } => {
                    if self.n == 0 {
                        r.skipped += 1;
                        continue;
                    }
                    self.give(pick as usize % self.n)?;
                    r.given += 1;
                }
                Op::GiveAll => {
                    for k in (0..self.n).rev() {
                        self.give(k)?;
                        r.given += 1;
                    }
                }
            }
        }
        // 收尾对账：本趟在册的都还回去（`given` 要计满 —— 并发下"取还不平"是硬判据）。
        for k in (0..self.n).rev() {
            self.give(k)?;
            r.given += 1;
        }
        Ok(r)
    }

    /// 归还一块：先读回模式字节（跨线程重叠/簿记踩踏的**主判据**），再放格、再归还。
    fn give(&mut self, k: usize) -> Result<(), Violation> {
        let l = self.live[k].expect("在册下标必在界内");
        for i in 0..l.len {
            // SAFETY: 该区间是本趟从分配器拿到的交付区，尚未归还。
            let got = unsafe { *l.ptr.as_ptr().add(i) };
            if got != l.tag {
                registry_release(l.slot);
                LIVE.fetch_sub(1, Ordering::Relaxed);
                let _ = self.remove(k);
                return Err(Violation::Pattern {
                    backend: l.backend,
                    addr: l.addr,
                    at: i,
                    want: l.tag,
                    got,
                });
            }
        }
        registry_release(l.slot);
        LIVE.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: 同一次 allocate 的产物，layout 原样带回。
        unsafe { l.backend.allocator().deallocate(l.ptr, l.layout) };
        self.remove(k);
        Ok(())
    }

    /// 摘掉一格（swap_remove：在册集合不变，顺序会变）。
    fn remove(&mut self, k: usize) -> Option<Live> {
        let out = self.live[k];
        if let Some(l) = out {
            self.live[k] = self.live[self.n - 1];
            self.live[self.n - 1] = None;
            self.n -= 1;
            self.held -= l.len;
        }
        out
    }

    /// 收尾/失败清理：把在册的都还回去（模式不符也照还 —— 已经要报缺陷了，别把台面带脏）。
    fn cleanup(&mut self) {
        while self.n > 0 {
            let l = self.live[self.n - 1].expect("末尾必在册");
            registry_release(l.slot);
            LIVE.fetch_sub(1, Ordering::Relaxed);
            // SAFETY: 同一次 allocate 的产物。
            unsafe { l.backend.allocator().deallocate(l.ptr, l.layout) };
            self.live[self.n - 1] = None;
            self.n -= 1;
            self.held -= l.len;
        }
    }
}

/// 在**并发**下跑一份计划（影子账 = 全局原子登记表）。
///
/// `budget` 是**本线程**的在册字节上限：并发时台面是共用的，各线程的预算要除以线程数
/// （调用方给，见 [`concurrent_churn`]）。
pub fn run_plan(ops: &[Op], budget: usize) -> Result<Report, Violation> {
    Run::new().go(ops, budget)
}

// ── 场景一：并发 churn（随机计划 × N 线程）──────────────────────────────────

/// `threads` 个线程各跑一份（LCG 生成的）随机计划，**同一起跑线**（`Barrier`）真并发。
///
/// 线程 → hart 的绑定是确定的（`t % hart_count`）：hart 数（4）小于线程数时，两个线程
/// 落在同一个池上（同池 push）与落在不同池上（跨池 feed）**两种路径都会走到**。
pub fn concurrent_churn(
    threads: usize,
    bytes_per_thread: usize,
    seed: u64,
) -> Result<Vec<Report>, Violation> {
    assert!(
        (1..=MAX_THREADS).contains(&threads),
        "线程数必须在 1..={MAX_THREADS}（登记表容量约束）"
    );
    assert!(threads * MAX_LIVE <= SLOTS, "登记表放不下这么多在册块");
    let budget = harness::Limits::STRICT.budget / threads;
    let start = Barrier::new(threads);

    let per_thread: Vec<Result<Report, Violation>> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            // 计划在**线程外**生成（宿主分配不进并发段，读数才归给分配器）。
            let plan = harness::plan(&harness::lcg_bytes(
                seed ^ (t as u64) << 17,
                bytes_per_thread,
            ));
            let start = &start;
            handles.push(s.spawn(move || {
                let harts = machine::hart_count().max(1);
                machine::hart_bind(t % harts);
                start.wait(); // 同一起跑线：真并发，不是排队
                run_plan(&plan, budget)
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("工作线程 panicked"))
            .collect()
    });

    // 按线程号归位，并把**第一处**违例上抛（其余线程的违例记在报告里，不再掩盖主判词）。
    let mut reports = vec![Report::default(); threads];
    let mut first: Option<Violation> = None;
    for (t, r) in per_thread.into_iter().enumerate() {
        match r {
            Ok(rep) => reports[t] = rep,
            Err(v) => {
                if first.is_none() {
                    first = Some(v);
                }
            }
        }
    }
    match first {
        Some(v) => Err(v),
        None => Ok(reports),
    }
}

// ── 场景二：跨 hart 归还（feed → suck）──────────────────────────────────────

/// 跨 hart 归还场景的读数。
#[derive(Clone, Copy, Debug)]
pub struct Handoff {
    /// hart 0 取到的块归属哪个池。
    pub pool_a: usize,
    /// hart 1 取到的块归属哪个池。
    pub pool_b: usize,
    /// 交回 hart 0 之后，hart 0 再取到的块里**仍归属本池**的块数。
    pub returned_home: usize,
    /// 每边的块数。
    pub blocks: usize,
}

/// **跨 hart 归还**：hart 0 取 `blocks` 块 → 交给 hart 1 归还（`home != me` ⇒ 走 `feed`）
/// → hart 0 再取，必须仍拿到**归属自己池**的块（本池 `pull` 时 `suck` 把泵里的块抽回池）。
///
/// 这是块池"块只进归属池的 freelist"这条硬不变量的**并发**版本：泵是唯一回路，
/// 跨核归还若丢了块（或塞错池），hart 0 再取就会拿到别的池的块（或取不到）。
pub fn cross_hart_handoff(blocks: usize) -> Result<Handoff, Violation> {
    /// `BLOCK_REQS[8] = (256, 16)`：块级请求，跨核归还走的正是块池的池归属路由。
    const REQ: u8 = 8;

    let (layout, want) = harness::request(Backend::Block, REQ);
    assert_eq!(want, Want::Ok, "请求表里的这一条必须合法");
    let (tx, rx) = std::sync::mpsc::channel::<(usize, usize, core::alloc::Layout, u8)>();
    let freed = Barrier::new(2);
    // 两个工作线程各自持一份 `&Barrier`（引用是 `Copy`）：栅栏本身留在本函数栈上，
    // 由 `thread::scope` 借出去（作用域线程不要求 `'static`）。
    let freed = &freed;

    let (a, b) = std::thread::scope(|s| {
        let ha = s.spawn(move || -> Result<(usize, usize), Violation> {
            machine::hart_bind(0);
            let alloc = block::allocator();
            let heap = block::heap();
            let mut mine: Vec<(usize, usize, core::alloc::Layout, u8)> = Vec::with_capacity(blocks);
            for i in 0..blocks {
                let buf = alloc.allocate(layout).map_err(|_| Violation::Rejected {
                    backend: Backend::Block,
                    size: layout.size(),
                    align: layout.align(),
                })?;
                let ptr = buf.cast::<u8>();
                let addr = ptr.as_ptr() as usize;
                let len = buf.len();
                let tag = (i as u8).wrapping_mul(TAG_MUL).wrapping_add(TAG_ADD);
                // SAFETY: 本指针本步刚拿到，`len` 是声明的交付长度。
                unsafe { core::ptr::write_bytes(ptr.as_ptr(), tag, len) };
                mine.push((addr, len, layout, tag));
            }
            let pool = heap.own(mine[0].0).expect("块级的块必然归属某个池");
            for &(addr, len, layout, tag) in &mine {
                // 交给 hart 1 归还（通道的另一端在 hart 1 上）。
                tx.send((addr, len, layout, tag)).expect("通道另一端在");
            }
            drop(mine);
            // 等 hart 1 把这批块**全部**归还（跨核 feed 完成）。
            freed.wait();
            // 再取同样多的块：必须仍归属**本**池。
            let mut home = 0usize;
            let mut again: Vec<(NonNull<u8>, core::alloc::Layout)> = Vec::with_capacity(blocks);
            for _ in 0..blocks {
                let buf = alloc.allocate(layout).map_err(|_| Violation::Rejected {
                    backend: Backend::Block,
                    size: layout.size(),
                    align: layout.align(),
                })?;
                let p = buf.cast::<u8>();
                if heap.own(p.as_ptr() as usize) == Some(pool) {
                    home += 1;
                }
                again.push((p, layout));
            }
            for (p, l) in again {
                // SAFETY: 同一次 allocate 的产物，layout 原样带回。
                unsafe { alloc.deallocate(p, l) };
            }
            Ok((pool, home))
        });

        let hb = s.spawn(move || -> Result<usize, Violation> {
            machine::hart_bind(1);
            let alloc = block::allocator();
            let heap = block::heap();
            // 先取一块**自己的**：登记 per-hart 池真的分家（这一块的归属应是池 1）。
            // （先前这里直接读"收到的第一块"归属，那是 hart 0 的块 —— 当然还是池 0。）
            let mine = alloc.allocate(layout).map_err(|_| Violation::Rejected {
                backend: Backend::Block,
                size: layout.size(),
                align: layout.align(),
            })?;
            let mine_ptr = mine.cast::<u8>();
            let pool = heap.own(mine_ptr.as_ptr() as usize);
            // SAFETY: 本指针本步刚拿到，layout 原样带回。
            unsafe { alloc.deallocate(mine_ptr, layout) };

            // 再收 hart 0 的块并归还（跨 hart ⇒ `feed`）。
            for _ in 0..blocks {
                let (addr, len, layout, tag) = rx.recv().expect("通道另一端在");
                // 交付区模式读回：hart 0 写进去的 tag 必须还在（第三方写入 = 重叠交付/簿记踩踏）。
                for i in 0..len {
                    // SAFETY: 该区间是 hart 0 刚交付的、本线程持有所有权的块。
                    let got = unsafe { *(addr as *const u8).add(i) };
                    if got != tag {
                        return Err(Violation::Pattern {
                            backend: Backend::Block,
                            addr,
                            at: i,
                            want: tag,
                            got,
                        });
                    }
                }
                // 跨 hart 归还：home(池) != me(hart 1) ⇒ `BlockAllocator::deallocate` 走 feed。
                // SAFETY: 同一次 allocate 的产物（所有权已随通道转给本线程）。
                unsafe { alloc.deallocate(NonNull::new(addr as *mut u8).expect("非空"), layout) };
            }
            freed.wait();
            Ok(pool.expect("至少一块"))
        });

        (
            ha.join().expect("hart 0 线程 panicked"),
            hb.join().expect("hart 1 线程 panicked"),
        )
    });

    let (pool_a, returned_home) = a?;
    let pool_b = b?;
    Ok(Handoff {
        pool_a,
        pool_b,
        returned_home,
        blocks,
    })
}

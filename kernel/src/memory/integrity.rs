
//! memory::integrity — 内存完整性检测框架（debug-only；release 整体编译为空）。
//!
//! 隐喻：Banker（页金库）+ Ledger（活块账本），同属簿记域。
//!   Banker — 页级占位：debit(pa) 取出（Free→held），credit(pa) 存入（held→Free）。
//!   Ledger — 活块登记：mark 入账 / unmark 校验+注销 / verify 任意地址 drop-in。
//!   poison — 毒化填充（未初始化读 / UAF 读数的现行标记）。
//!   report — 统一处置：trace 记事件 → 现场直写 → panic（halt 再转储 crash scene）。
//!   stats  — 基线核算查询。audit — Banker↔Ledger↔frame↔block 交叉核对。
//!
//! 收编：pageown（页级堆持有 1-bit）→ Banker 全池占位；unitmap（块级在位 8B 单元
//! 位图）→ Ledger 按地址存在性（重复入账=双发、尺寸不符=错幂释放、清页残留=记账错）。
//!
//! 硬不变量（贴结构）：
//!   - Ledger 容量 init 预留（with_capacity）；mark 在 soft_cap 内插入**零分配**——
//!     绝不持锁触碰分配器（防 block 重入 / 锁序死锁）。
//!   - Banker 无锁（原子位图）；Ledger 锁 = Level::Ledger（层级 7），只在无锁或
//!     低层级锁内获取（Space=2 / 无锁路径），绝不反向嵌套。
//!   - canary 只写 KernelHeap 块（用户堆为清零语义，不 poison、不 canary）。

#![cfg(debug_assertions)] // 完整性框架整体 debug-only（release 编译为空，零开销）

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use alloc::boxed::Box;
use hashbrown::HashMap;

use crate::console::_write;
use crate::lock::{Level, OnceLock, SpinLock};
use crate::memory::PAGE_SIZE;
use crate::runtime::trace;

/// 毒化模式字节（分配/释放填充：未初始化读、UAF 读数的现行标记）。
pub const POISON: u8 = 0xCD;
/// slack canary 期望值（写进块尾 slack 8 字节；释放时核对）。
const CANARY_MAGIC: u64 = 0x51A7_0D1E_CAFE_BEEF;
/// canary 所需最小 slack 字节数（不足则本块不设 canary）。
const CANARY_MIN_SLACK: usize = 8;

/// 登记类别（Ledger 记录归属；用户堆不 poison/canary——维持清零语义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    KernelHeap,
    UserHeap,
}

/// 完整性违例类别（report 的字段；repr(u8) 供 trace 事件编码，**顺序即 ABI**）。
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum IntegrityViolation {
    /// Banker::debit 作用于已 held 页（双取出）。
    DoubleDebit = 0,
    /// Banker::credit 作用于 Free 页（存入陌生页）。
    DoubleCredit = 1,
    /// unmark/verify 遇到无账地址（双 free / 悬垂指针 / 野指针）。
    UnregisteredFree = 2,
    /// 地址越 Ledger 所属域（如 KernelHeap 记录不在任何块池区段）。
    WildAddress = 3,
    /// 重复入账（块级双发）。
    DuplicateMark = 4,
    /// slack canary 被覆写（越界写现行）。
    CanaryBroken = 5,
    /// 释放尺寸 ≠ 登记尺寸（错幂释放 / 脏指针）。
    SizeMismatch = 6,
    /// 侧表容量耗尽（debug 资源上限，非内存耗尽）。
    LedgerOom = 7,
    /// 登记/查询先于 init。
    NotInitialized = 8,
    /// 多源交叉核对不一致（audit / 页清残留）。
    AuditDivergence = 9,
}

// ── Banker：页金库占位（无锁原子位图；覆盖帧池整个 free 区）────────────

pub struct Banker {
    base: AtomicUsize,
    words: OnceLock<Box<[AtomicU64]>>,
}

impl Banker {
    pub const fn new() -> Banker {
        Banker {
            base: AtomicUsize::new(0),
            words: OnceLock::new(),
        }
    }

    /// 登记金库范围（block::init 调用恰好一次；须先于任何 debit/credit）。
    pub fn init(&self, base: usize, pages: usize) {
        self.base.store(base, Ordering::Relaxed);
        let words = pages.div_ceil(64);
        let table: Box<[AtomicU64]> = (0..words).map(|_| AtomicU64::new(0)).collect();
        assert!(self.words.set(table).is_ok(), "banker double init");
    }

    fn idx(&self, pa: usize) -> usize {
        let base = self.base.load(Ordering::Relaxed);
        assert!(pa >= base, "banker: page {pa:#x} below base {base:#x}");
        let n = (pa - base) / PAGE_SIZE;
        let bits = self.words.get().expect("banker not initialized").len() * 64;
        assert!(n < bits, "banker: page {pa:#x} outside bank (base {base:#x})");
        n
    }

    fn bit(&self, pa: usize) -> bool {
        let n = self.idx(pa);
        self.words.get().expect("banker not initialized")[n / 64].load(Ordering::Relaxed)
            & (1u64 << (n % 64))
            != 0
    }

    /// 取出页：Free → held。前置：页在库且为 Free（双取出现行）。
    #[track_caller]
    pub fn debit(&self, pa: usize) {
        if self.bit(pa) {
            report(
                IntegrityViolation::DoubleDebit,
                pa,
                format_args!("debit on already-held page"),
            );
        }
        let n = self.idx(pa);
        self.words.get().expect("banker not initialized")[n / 64]
            .fetch_or(1u64 << (n % 64), Ordering::Relaxed);
    }

    /// 存入页：held → Free。前置：页确为 held（存入陌生页现行）。
    #[track_caller]
    pub fn credit(&self, pa: usize) {
        if !self.bit(pa) {
            report(
                IntegrityViolation::DoubleCredit,
                pa,
                format_args!("credit on free page"),
            );
        }
        let n = self.idx(pa);
        self.words.get().expect("banker not initialized")[n / 64]
            .fetch_and(!(1u64 << (n % 64)), Ordering::Relaxed);
    }

    pub fn is_held(&self, pa: usize) -> bool {
        self.bit(pa)
    }

    /// 在途页总数（held 位 popcount；stats/audit 用）。
    pub fn held_count(&self) -> usize {
        self.words
            .get()
            .expect("banker not initialized")
            .iter()
            .map(|w| w.load(Ordering::Relaxed).count_ones() as usize)
            .sum()
    }
}

/// 金库全局单例（block::init 装配）。
pub static BANKER: Banker = Banker::new();

// ── Ledger：活块账本（hashbrown；容量 init 预留，运行期插入零分配）────────

/// 一条活块登记。
pub struct Record {
    /// 块类（2 的幂字节；canary slack 判定）。
    class: usize,
    /// 登记时请求字节数（canary 位置 = addr + size；unmark 校 SizeMismatch）。
    size: usize,
    /// 分配点返回地址（alloc-site，violation 报告转储）。
    pub(crate) site: usize,
    /// slack canary（Some = 在 addr+size 处写入 8 字节；UserHeap 恒 None）。
    canary: Option<u64>,
    kind: OwnerKind,
}

pub struct Ledger {
    /// (map, soft_cap)：soft_cap = 容量 × 7/8——len < soft_cap 时 hashbrown 插入
    /// 装载 < 0.875 不扩容、零分配（见模块头硬不变量）。
    inner: SpinLock<Option<(HashMap<usize, Record>, usize)>>,
}

impl Ledger {
    pub const fn new() -> Ledger {
        Ledger {
            inner: SpinLock::new_level(Level::Ledger, None),
        }
    }

    /// 预留容量（block::init 调用恰好一次；先于任何 mark/unmark）。
    pub fn init(&self, capacity: usize) {
        let cap = capacity.max(64);
        let soft = cap / 8 * 7;
        *self.inner.lock() = Some((HashMap::with_capacity(cap), soft));
    }

    /// 活块入账。前置：已 init、容量充足、地址未登记（DuplicateMark 现行）。
    /// KernelHeap 且 slack ≥ 8 时顺带写 slack canary。**零分配**。
    pub fn mark(&self, addr: usize, size: usize, site: usize, kind: OwnerKind) {
        let mut g = self.inner.lock();
        let Some((map, soft)) = g.as_mut() else {
            report(IntegrityViolation::NotInitialized, addr, format_args!("ledger mark before init"));
        };
        if map.len() >= *soft {
            report(
                IntegrityViolation::LedgerOom,
                addr,
                format_args!("soft cap {soft} reached"),
            );
        }
        if map.contains_key(&addr) {
            report(
                IntegrityViolation::DuplicateMark,
                addr,
                format_args!("site {site:#x}"),
            );
        }
        let class = size.max(8).next_power_of_two();
        // canary 槽位按 8 对齐（块首 + 请求尺寸可能不对齐；u64 读写必须对齐——
        // 未对齐会触发 misaligned trap 进 OpenSBI 模拟，极慢/卡死）。
        let aligned = (size + 7) & !7;
        let canary = (kind == OwnerKind::KernelHeap && class - aligned >= CANARY_MIN_SLACK).then(|| {
            let at = addr + aligned;
            // SAFETY: at..at+8 落在块 slack 区（class ≥ aligned+8），块此刻独占（分配未交付）。
            unsafe { (at as *mut u64).write_volatile(CANARY_MAGIC) };
            CANARY_MAGIC
        });
        map.insert(
            addr,
            Record {
                class,
                size,
                site,
                canary,
                kind,
            },
        );
    }

    /// 任意地址 drop-in 校验：无账 → report(UnregisteredFree)；有账 → canary 核对。
    pub fn verify(&self, addr: usize) {
        let g = self.inner.lock();
        let Some((map, _)) = g.as_ref() else {
            report(IntegrityViolation::NotInitialized, addr, format_args!("ledger verify before init"));
        };
        let Some(rec) = map.get(&addr) else {
            report(IntegrityViolation::UnregisteredFree, addr, format_args!("verify: no record"));
        };
        check_canary(addr, rec);
    }

    /// 唯一注销入口：先证（存在 + canary 完好 + 尺寸一致）再移除；移除后该地址即「无账」。
    pub fn unmark(&self, addr: usize, size: usize) {
        let mut g = self.inner.lock();
        let Some((map, _)) = g.as_mut() else {
            report(IntegrityViolation::NotInitialized, addr, format_args!("ledger unmark before init"));
        };
        let rec = map.get(&addr).unwrap_or_else(|| {
            report(IntegrityViolation::UnregisteredFree, addr, format_args!("unmark: no record"));
        });
        if rec.size != size {
            check_canary(addr, rec);
            report(
                IntegrityViolation::SizeMismatch,
                addr,
                format_args!("record size {} vs freed {size}", rec.size),
            );
        }
        check_canary(addr, rec);
        map.remove(&addr);
    }

    /// 锁内遍历（audit 用）；回调签名 (addr, &Record)。
    pub fn for_each(&self, mut f: impl FnMut(usize, &Record)) {
        let g = self.inner.lock();
        let Some((map, _)) = g.as_ref() else { return };
        for (addr, rec) in map.iter() {
            f(*addr, rec);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().as_ref().map(|(m, _)| m.len()).unwrap_or(0)
    }
}

/// 全局账本单例（block::init 装配）。
pub static LEDGER: Ledger = Ledger::new();

/// 核对记录 canary（未设则该页无动作）。
fn check_canary(addr: usize, rec: &Record) {
    if let Some(magic) = rec.canary {
        // 与 mark 同表达式：对齐后的 slack 槽位（见 mark 注释）。
        let at = (addr + rec.size + 7) & !7;
        // SAFETY: canary 只对 KernelHeap 登记（对齐 slack 区，≤ 块尾），释放前块仍独占。
        let got = unsafe { (at as *const u64).read_volatile() };
        if got != magic {
            report(
                IntegrityViolation::CanaryBroken,
                addr,
                format_args!("canary @{at:#x} = {got:#x} != {magic:#x}"),
            );
        }
    }
}

// ── 毒化 ──────────────────────────────────────────────

/// 毒化填充 [addr, addr+len)。前置：区间此刻归调用方独占（刚分配未交付 / 已取出未复用）。
pub fn poison(addr: usize, len: usize) {
    // SAFETY: 前置条件保证区间独占可写；volatile 写防写合并吞掉标记。
    let p = addr as *mut u8;
    for i in 0..len {
        unsafe { p.add(i).write_volatile(POISON) };
    }
}

// ── 统一处置 ──────────────────────────────────────────

/// 统一处置（不返回）：trace 记 Mem(Integrity) → 现场直写 → panic
/// （halt 的 panic 处理器再转储 crash scene；panic 路径零分配）。
pub fn report(v: IntegrityViolation, addr: usize, detail: fmt::Arguments) -> ! {
    trace::note(trace::EventKind::Mem(trace::MemEvent::Integrity {
        code: v as u8,
        addr,
    }));
    _write(format_args!("[integrity] {v:?} at {addr:#x}: {detail}
"));
    panic!("memory integrity violation: {v:?}");
}

// ── 基线核算 ──────────────────────────────────────────

/// 基线核算快照（check_baseline / 启动横幅用）。
pub struct IntegrityStats {
    pub held_frames: usize,
    pub kernel_blocks: usize,
    pub user_blocks: usize,
}

pub fn stats() -> IntegrityStats {
    let mut kernel_blocks = 0usize;
    let mut user_blocks = 0usize;
    LEDGER.for_each(|_, rec| match rec.kind {
        OwnerKind::KernelHeap => kernel_blocks += 1,
        OwnerKind::UserHeap => user_blocks += 1,
    });
    IntegrityStats {
        held_frames: BANKER.held_count(),
        kernel_blocks,
        user_blocks,
    }
}

// ── 审计 ──────────────────────────────────────────────

/// 页内是否有活账目（decrease_used 整页清链后的记账完整性检查）。
pub fn page_has_records(pa: usize) -> bool {
    let mut any = false;
    LEDGER.for_each(|addr, _| {
        if addr >= pa && addr < pa + PAGE_SIZE {
            any = true;
        }
    });
    any
}

/// 全量审计（boot 收尾调用一次；三源交叉核对，违例即 report）：
///   Banker.held_count == frame.outstanding + block.torn_pages；
///   每条 KernelHeap 记录地址须落在块池区段，且所在页须 held（撕页时已 debit）。
pub fn audit() {
    let held = BANKER.held_count();
    let frames = crate::memory::allocator::frame::FRAME_ALLOCATOR.outstanding();
    let torn = crate::memory::allocator::block::torn_pages();
    if held != frames + torn {
        report(
            IntegrityViolation::AuditDivergence,
            0,
            format_args!("banker {held} != frames {frames} + torn {torn}"),
        );
    }
    LEDGER.for_each(|addr, rec| {
        let page = addr & !(PAGE_SIZE - 1);
        if rec.kind == OwnerKind::KernelHeap {
            if !crate::memory::allocator::block::pool_includes(addr) {
                report(IntegrityViolation::WildAddress, addr, format_args!("kernel-heap record outside pool segments"));
            }
            if !BANKER.is_held(page) {
                report(
                    IntegrityViolation::AuditDivergence,
                    addr,
                    format_args!("kernel-heap record on non-held page {page:#x}"),
                );
            }
        } else if !BANKER.is_held(page) {
            report(
                IntegrityViolation::AuditDivergence,
                addr,
                format_args!("user-heap record VA on non-held page {page:#x}"),
            );
        }
    });
}

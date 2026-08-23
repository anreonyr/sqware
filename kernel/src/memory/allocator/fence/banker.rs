//! 护栏层 · banker — 页金库占位（无锁原子位图，覆盖帧池整个 free 区）
//!
//! 每页 1 bit：debit(pa) 取出（Free→held）、credit(pa) 存入（held→Free）。无锁
//! （原子 Relaxed——位图自占位即同步，不需要扩展开销）。范围由 `init` 装配，
//! 先于任何 debit/credit。违例统一经 `report` 处置（见 fence/mod）。
//!
//! 硬不变量（贴结构）：debit 前置 held==false（双取出 = 违例）；credit 前置
//! held==true（存陌生页 = 违例）——表项是「谁借了这页」的唯一权威。

#![cfg(all(debug_assertions, feature = "audit"))] // 与 fence 根同 gate（debug + audit）

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use alloc::boxed::Box;

use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;

use super::{IntegrityViolation, report};

/// 页金库：free 区每页 1 bit 的在途（held）位图。
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
        assert!(
            n < bits,
            "banker: page {pa:#x} outside bank (base {base:#x})"
        );
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

    /// 在途页总数（held 位 popcount；随 stats/audit 用）。
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
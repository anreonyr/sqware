//! 护栏层 · ledger — 活块账本（hashbrown；容量 init 预留，运行期插入零分配）
//!
//! 按地址登记在册活块（mark 入账、unmark 校验+注销、verify 任意地址 drop-in、
//! for_each 锁内遍历、sweep_canaries 崩溃现场清查）。笼统：
//!   - 容量 init 预留（with_capacity），soft_cap = 容量 × 7/8——插入在装载 < 0.875
//!     不扩容、零分配（绝不持锁触碰分配器，防 block 重入 / 锁序死锁）；
//!   - 锁 = Level::Ledger（层级 7，只在无锁或低层级锁内获取，持锁**绝不分配**——
//!     audit 只读块归属（pool_includes → tally，层级 8 更高）不受限；
//!     插入（mark）容量 init 预留、零分配、绝不反向嵌套）；
//!   - canary 只写 KernelHeap 块（用户堆为清零语义，不 poison、不 canary）。
//! 违例统一经 `report` 处置（见 fence/mod）。

#![cfg(all(debug_assertions, feature = "audit"))] // 与 fence 根同 gate（debug + audit）

use hashbrown::HashMap;

use crate::lock::{Level, SpinLock};

use super::{CANARY_MAGIC, CANARY_MIN_SLACK, IntegrityViolation, report};

/// 登记类别（Ledger 记录归属；用户堆不 poison/canary——维持清零语义）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    KernelHeap,
    UserHeap,
}

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
    /// 登记类别（audit 交叉核对用；audit.rs 读）。
    pub(crate) kind: OwnerKind,
}

/// 活块账本：地址 → 记录（锁内；容量 init 预留，运行期零分配）。
pub struct Ledger {
    /// (map, soft_cap)：len < soft_cap 时 hashbrown 插入装载 < 0.875 不扩容、零分配。
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
            report(
                IntegrityViolation::NotInitialized,
                addr,
                format_args!("ledger mark before init"),
            );
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
        let canary =
            (kind == OwnerKind::KernelHeap && class - aligned >= CANARY_MIN_SLACK).then(|| {
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
            report(
                IntegrityViolation::NotInitialized,
                addr,
                format_args!("ledger verify before init"),
            );
        };
        let Some(rec) = map.get(&addr) else {
            report(
                IntegrityViolation::UnregisteredFree,
                addr,
                format_args!("verify: no record"),
            );
        };
        check_canary(addr, rec);
    }

    /// 唯一注销入口：先证（存在 + canary 完好 + 尺寸一致）再移除；移除后该地址即「无账」。
    pub fn unmark(&self, addr: usize, size: usize) {
        let mut g = self.inner.lock();
        let Some((map, _)) = g.as_mut() else {
            report(
                IntegrityViolation::NotInitialized,
                addr,
                format_args!("ledger unmark before init"),
            );
        };
        let rec = map.get(&addr).unwrap_or_else(|| {
            report(
                IntegrityViolation::UnregisteredFree,
                addr,
                format_args!("unmark: no record"),
            );
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

    /// 活块 canary 清查（panic 现场 drop-in；只报不 panic——诊断不截断转储）。
    ///
    /// 平时 canary 只在释放（[`unmark`]）时核对；**永不释放的泄漏块**（符号表
    /// 等）的 canary 永远等不到检查，且越界写若只砸进相邻**活**块，期间无任何
    /// 告警。崩溃现场扫一遍全部 KernelHeap 记录：砸坏的 canary → 打印受害块
    /// 地址与分配点返回地址（alloc-site，host addr2line 可符号化）并计数。
    /// try_lock：现场若正持 Ledger 锁（如分配器内 panic）则跳过、静默。返回
    /// 砸坏数。本模块整体 gate（debug+audit），调用方同条件。
    pub fn sweep_canaries(&self) -> usize {
        let Some(g) = self.inner.try_lock() else {
            return 0;
        };
        let Some((map, _)) = g.as_ref() else {
            return 0;
        };
        let mut bad = 0usize;
        for (addr, rec) in map.iter() {
            if rec.canary.is_none() {
                continue; // UserHeap 不设 canary
            }
            // 与 mark/unmark 同表达式：对齐后的 slack 槽位（见 mark 注释）。
            let at = (addr + rec.size + 7) & !7;
            // SAFETY: canary 只对 KernelHeap 登记（对齐 slack 区，≤ 块尾）；崩溃
            // 现场只读。若该处已被越界写砸坏，值 ≠ magic——正是本清查要找的。
            let got = unsafe { (at as *const u64).read_volatile() };
            if got != CANARY_MAGIC {
                crate::putln!(
                    "[integrity] sweep: canary broken @{at:#x} (block {addr:#x} site {:#x})",
                    rec.site
                );
                bad += 1;
            }
        }
        bad
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .as_ref()
            .map(|(m, _)| m.len())
            .unwrap_or(0)
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
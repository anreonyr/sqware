// 护栏层（fence）— 内存运行时不变量检查，内嵌在生产路径（in-path）
//
// 与「自测（selftest，out-of-path 验收用例）」相对：护栏是功能运行的自我证明，
// 命中即 halt（panic → crash scene）。四个成员：
//   checker  — 分配器链式不变式断言（block/frame 的 freepool 判重、越界、环与
//              流水观测）。钩子恒编译、单行调用，release 空体零开销（debug gate）。
//   banker   — 页金库占位（无锁原子位图；free 区每页 1 bit，debit/credit/is_held）。
//   ledger   — 活块账本（hashbrown 登记表；mark/unmark/verify/canary，锁内零分配）。
//   audit    — 核查侧（多源交叉核对 audit()、关机基线 record/check_baseline、
//              页清残留 page_clear、基线快照 stats）。
// 模块根 = banker/ledger/checker/audit 共享的处置原语：report（违例→trace→panic）、
// IntegrityViolation（违例类目）、poison（毒化标记）、OwnerKind（登记类别）与相关
// 常量，以及**事件入口**（on_alloc/on_free/on_frame_alloc/on_frame_free）——
// 分配器热路径对其的调用是一行无 cfg 的语义事件，asm 读 ra、poison、记账全部
// 收在本层内部（纯功能文件零审计词汇）。
//
// 依赖方向（无环）：checker 独立；banker/ledger → 模块根；audit → 模块根 + banker + ledger。

#![allow(unused)]
use alloc::fmt;

pub mod audit;
pub mod banker;
pub mod checker;
pub mod ledger;

// ── 公共处置原语（debug-gated；release 编译为空，零开销）──

/// 毒化模式字节（分配/释放填充：未初始化读、UAF 读数的现行标记）。
#[cfg(debug_assertions)]
pub const POISON: u8 = 0xCD;
/// slack canary 期望值（写进块尾 slack 8 字节；释放时核对）。
#[cfg(debug_assertions)]
pub(crate) const CANARY_MAGIC: u64 = 0x51A7_0D1E_CAFE_BEEF;
/// canary 所需最小 slack 字节数（不足则本块不设 canary）。
#[cfg(debug_assertions)]
pub(crate) const CANARY_MIN_SLACK: usize = 8;

/// 登记类别（Ledger 记录归属；用户堆不 poison/canary——维持清零语义）。
/// 放模块根（不 gate）：事件入口签名引用它，release 下 ledger 模块为空也可编译。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    KernelHeap,
    UserHeap,
}

/// 完整性违例类别（report 的字段；repr(u8) 供 trace 事件编码，**顺序即 ABI**）。
#[cfg(debug_assertions)]
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

/// 毒化填充 [addr, addr+len)。前置：区间此刻归调用方独占（刚分配未交付 / 已取出未复用）。
///
/// SAFETY: 前置条件保证区间独占可写；volatile 写防写合并吞掉标记。
#[cfg(debug_assertions)]
pub fn poison(addr: usize, len: usize) {
    // SAFETY: 调用方保证区间独占；volatile 写。
    let p = addr as *mut u8;
    for i in 0..len {
        unsafe { p.add(i).write_volatile(POISON) };
    }
}

/// 统一处置（不返回）：trace 记 Mem(Integrity) → 现场直写 → panic
/// （halt 的 panic 处理器再转储 crash scene；panic 路径零分配）。
#[cfg(debug_assertions)]
pub fn report(v: IntegrityViolation, addr: usize, detail: fmt::Arguments) -> ! {
    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Memory(
        crate::runtime::diagnose::trace::MemoryEvent::Integrity {
            code: v as u8,
            addr,
        },
    ));
    crate::console::_write(format_args!(
        "[integrity] {v:?} at {addr:#x}: {detail}
"
    ));
    panic!("memory integrity violation: {v:?}");
}

// ── 事件入口（恒编译；体内 debug-gated，release 空体零开销）──
//
// 分配器热路径唯一可见的护栏痕迹：一行语义调用。asm 读 ra、poison 填充、
// ledger/banker 记账全部收在本层内部——纯功能文件（block/frame/space）不见
// 任何 cfg、编译器内联指令或审计词汇。debug 构建做账，release 空体经
// #[inline] 消除。

/// 用户堆活块账键（**跨模块硬不变量：键必须单射**）。
///
/// `(asid << 44) | (va >> 12)`：asid < 2^16，va < 2^56（任何受支持模式的用户
/// 半区）→ 页索引 < 2^44，两段不重叠。替代旧 `asid<<32|va`（VA≥2^32 时碰撞）。
///
/// 恒编译（release 返回 0 空体，与事件入口同惯例）。
#[inline]
pub(crate) fn key(asid: usize, va: usize) -> usize {
    #[cfg(debug_assertions)]
    {
        (asid << 44) | (va >> 12)
    }
    #[cfg(not(debug_assertions))]
    {
        0
    }
}

/// 分配事件：活块入账（KernelHeap 整块毒化）。caller = 调用点 ra（分配现场符号化用）。
/// 用户堆不 poison（键 = [`key`] 页索引编码，非地址；且用户页维持清零语义，见
/// [`OwnerKind`]）——只入 ledger 账。
#[inline]
pub fn on_alloc(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(debug_assertions)]
    {
        if let OwnerKind::KernelHeap = kind {
            poison(addr, size);
        }
        let caller: usize;
        // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
        unsafe { core::arch::asm!("mv {}, ra", out(reg) caller) };
        ledger::LEDGER.mark(addr, size, caller, kind);
    }
}

/// 释放事件：活块注销 + KernelHeap 本体毒化复写（头 8B 随后被 freelist 头插
/// 覆盖，其余保持毒化——UAF 读数变 0xCD）。用户堆不 poison（同 [`on_alloc`]：
/// 键非地址、维持清零语义）——只注销账目。
#[inline]
pub fn on_free(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(debug_assertions)]
    {
        ledger::LEDGER.unmark(addr, size);
        if let OwnerKind::KernelHeap = kind {
            poison(addr, size);
        }
    }
}

/// 帧分配事件：页金库取出（Free→held；双取出 / 活堆页泄漏进池现行）。
#[inline]
pub fn on_frame_alloc(addr: usize) {
    #[cfg(debug_assertions)]
    {
        banker::BANKER.debit(addr);
    }
}

/// 帧释放事件：页金库存入（held→Free；存入陌生页 / 双释放现行）。
#[inline]
pub fn on_frame_free(addr: usize) {
    #[cfg(debug_assertions)]
    {
        banker::BANKER.credit(addr);
    }
}

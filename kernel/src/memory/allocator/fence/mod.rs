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
// IntegrityViolation（违例类目）、poison（毒化标记）与相关常量。
//
// 分层（in-path / out-of-path）：
//   功能   memory/allocator/{block,frame,hybrid,...} — 实现本身
//   护栏   memory/allocator/fence/*                    — 本层（in-path 检查）
//   自测   selftest/*                                  — 开机一次性验收（独立于功能）
//
// 依赖方向（无环）：checker 独立；banker/ledger → 模块根；audit → 模块根 + banker + ledger。

pub mod audit;
pub mod banker;
pub mod checker;
pub mod ledger;

use core::fmt;

// ── 公共处置原语（audit-gated；release/无 audit feature 编译为空）──

/// 毒化模式字节（分配/释放填充：未初始化读、UAF 读数的现行标记）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub const POISON: u8 = 0xCD;
/// slack canary 期望值（写进块尾 slack 8 字节；释放时核对）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) const CANARY_MAGIC: u64 = 0x51A7_0D1E_CAFE_BEEF;
/// canary 所需最小 slack 字节数（不足则本块不设 canary）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub(crate) const CANARY_MIN_SLACK: usize = 8;

/// 完整性违例类别（report 的字段；repr(u8) 供 trace 事件编码，**顺序即 ABI**）。
#[cfg(all(debug_assertions, feature = "audit"))]
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
#[cfg(all(debug_assertions, feature = "audit"))]
pub fn poison(addr: usize, len: usize) {
    // SAFETY: 调用方保证区间独占；volatile 写。
    let p = addr as *mut u8;
    for i in 0..len {
        unsafe { p.add(i).write_volatile(POISON) };
    }
}

/// 统一处置（不返回）：trace 记 Mem(Integrity) → 现场直写 → panic
/// （halt 的 panic 处理器再转储 crash scene；panic 路径零分配）。
#[cfg(all(debug_assertions, feature = "audit"))]
pub fn report(v: IntegrityViolation, addr: usize, detail: fmt::Arguments) -> ! {
    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Mem(
        crate::runtime::diagnose::trace::MemEvent::Integrity {
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
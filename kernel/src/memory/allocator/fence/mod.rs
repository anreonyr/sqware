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
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// 分配点回溯（诊断 site）：从当前 fp 沿标准 RV64 帧链上溯 depth 帧，取返回地址。
///
/// debug O0 调用链稳定：帧 0 = block 分配器（on_alloc 的调用者）、再上是
/// core::alloc 与装箱/容器/业务帧。core 预编译库（__rust_alloc 等）无 FP
/// （见 scene::kbacktrace 注释），链在彼处即断——断点前最后有效 ra 即 site。
///
unsafe extern "C" {
    /// .rodata 段起点（link.ld `_rodata_start`）：候选 ra 须落在其下（.text 区）
    /// ——.rodata 上的 vtable/常量指针是栈上常见数据，误收即 site 失真。
    static _rodata_start: u8;
}

/// .text 段上界（.rodata 起点）。
fn text_end() -> usize {
    // SAFETY: 链接脚本符号，恒存在。
    unsafe { (&raw const _rodata_start).addr() }
}

/// 分配点回溯（诊断 site）：从当前 fp 沿标准 RV64 帧链上溯 depth 帧；链在 core
/// 预编译库处断（无 FP，见 scene::kbacktrace 同款问题）后，从断点帧顶向上扫描
/// 收集候选 ra（4 对齐 + 镜像 .text + 非重复；连续 SCAN_GAP 字无候选 = 已越
/// 活跃帧区，停）。返回 (site, site2) = 扫描候选第 3、4 个（第 1、2 个 ≈ core
/// 帧保存 ra，无区分度）：候选序列 ≈ [core 帧 ra]×2、[装箱/容器帧 ra]、[业务
/// 帧 ra]、…——两条一并打印，离线 addr2line 择真。
///
/// 守卫轻量（热路径）：fp 单调增（栈向下生长，必终止）+ 同栈窗（链帧必在本栈，
/// 跨度 < 1 MiB——任务栈最大 256 KiB）+ 帧顶 16 对齐 + ra 落在 .text。读的
/// 是当前栈的调用者帧（执行必经过，帧已物化）——栈窗 VA 在**用户半区**（见
/// layout 栈窗），不可用 is_user 过滤。
///
/// 读侧逐页翻译（当前 satp 根，DRAM 守卫）：栈 slot 顶上是 guard/窗口边界
/// （未映射），扫描越界须停而非缺页 panic。SCAN_WINDOW 内最多两页，按页
/// 缓存翻译结果（每页一次 walk）。
#[cfg(debug_assertions)]
fn alloc_site(depth: usize) -> (usize, usize) {
    let mut fp: usize;
    // SAFETY: 读 s0 无副作用。
    unsafe { core::arch::asm!("mv {0}, s0", out(reg) fp) };
    let mut ra = 0usize;
    let mut prev = 0usize;
    for _ in 0..depth {
        // SAFETY: fp 指向当前栈的调用者帧（帧指针链），只读两个字。
        // 标准 RV64 布局：ra 在 [fp-8]、调用者 fp 在 [fp-16]（反汇编实证：
        // 序言 st ra, N-8(sp); st s0, N-16(sp); s0 = sp+N）。
        let (next, ret) = unsafe {
            (
                (fp as *const usize).sub(2).read_unaligned(),
                (fp as *const usize).sub(1).read_unaligned(),
            )
        };
        if next <= fp
            || next - fp > 0x10_0000
            || next & 0xF != 0
            || ret & 3 != 0
            || ret < 0x8020_0000
            || ret >= text_end()
        {
            break; // 脱链 / 出栈窗 / 非对齐 / 非 .text 可执行地址
        }
        ra = ret;
        prev = ret;
        fp = next;
    }
    // ② 启发式接续：链在 core 预编译库处断（无 FP），其外层业务帧仍在栈上——
    // 从断点帧顶向上逐字扫描收集候选 ra；收满 8 个或越活跃帧区即停。
    let mut a = fp;
    let mut gap = 0usize;
    let mut cands: [usize; 8] = [0; 8];
    let mut n = 0usize;
    let mut cache: Option<(usize, usize)> = None; // (va 页基, pa 页基)
    while a - fp < 0x4000 && gap < 96 && n < 8 {
        let page = a & !(crate::memory::PAGE_SIZE - 1);
        let pa_page = match cache {
            Some((p, pa)) if p == page => pa,
            _ => {
                let satp_val = riscv::register::satp::read().bits();
                let ppn = satp_val & ((1usize << 44) - 1);
                let in_dram = |pa: crate::memory::manager::addr::PhysAddr| {
                    (0x8000_0000..crate::machine::dram_edge().unwrap_or(0x9000_0000))
                        .contains(&pa.as_usize())
                };
                // SAFETY: walk_raw 只读页表（S 态当前根表），无副作用。
                let Some((pa0, flags)) = crate::memory::manager::table::TableNode::walk_raw(
                    crate::memory::manager::addr::PhysAddr::from_raw(ppn << 12),
                    crate::memory::manager::addr::VirtAddr::from_raw(page),
                    in_dram,
                ) else {
                    break; // 未映射页：越 slot 顶，停扫（不读不崩）
                };
                if !flags.contains(crate::memory::manager::entry::PteFlags::R) {
                    break;
                }
                let pa = pa0.as_usize();
                cache = Some((page, pa));
                pa
            }
        };
        // SAFETY: 页已翻译且 R 可读（上）；读侧 read_unaligned 无对齐 precondition。
        let w = unsafe { ((pa_page + (a - page)) as *const usize).read_unaligned() };
        if w & 3 == 0 && w >= 0x8020_0000 && w < text_end() && w != prev {
            cands[n] = w;
            n += 1;
            prev = w;
            gap = 0;
        } else {
            gap += 1;
        }
        a += 8;
    }
    // site = 候选第 5、6 个（跳过 4 层 core/分配器帧——实证候选序 ≈
    // [fmt::num, block:345, block:632, hybrid, 业务帧…]）；不足则回退链尾 ra。
    if n >= 5 {
        (cands[4], if n >= 6 { cands[5] } else { 0 })
    } else {
        (ra, 0)
    }
}

/// realloc 窗口（per-hart）：portal::grow 显式 begin/end 标记。grow 默认路径 =
/// 同 hart「allocate 新 → copy → deallocate 旧」——窗口内 on_alloc 记首笔新块、
/// on_free 见旧块即判定搬家（基线 rehome）而非错释。仅旧块在基线时才设窗
/// （普通对象 realloc 不涉及基线判定，零开销）。
///
/// 窗口内**关 SIE**（临界区微秒级）：S-timer 抢占会让同 hart 其它任务在窗口内
/// 分配——RALLOC_NEW 被覆盖即错配 rehome（已实证：rehome 落到无关块，基线
/// 失配误报 missing）。关中断后窗口内分配必属本 grow。
static IN_REALLOC: [AtomicBool; 16] = [const { AtomicBool::new(false) }; 16];
static RALLOC_OLD: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
static RALLOC_NEW: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
/// SIE 恢复标记（0 = 无需恢复；非 0 = begin 关过 SIE，end 恢复）。
static RALLOC_SIE: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];

/// 进入 realloc 窗口（portal::grow 调用；须与 [`end_realloc`] 配对）。
/// 旧块在基线才设窗——其 free 将 rehome 基线而非报错释。
#[cfg(debug_assertions)]
pub fn begin_realloc(old: usize) {
    let hart = crate::machine::hart_id().min(15) as usize;
    if crate::memory::allocator::fence::audit::is_baseline_block(old) {
        // 关 SIE：窗口内不可抢占（见 IN_REALLOC 注释——抢占污染配对）。
        let sie = riscv::register::sstatus::read().sie() as usize;
        // SAFETY: 关/开 SIE 仅改本 hart 中断使能位，窗口极短；end_realloc 恢复。
        unsafe { riscv::register::sstatus::clear_sie() };
        RALLOC_SIE[hart].store(sie, Ordering::Relaxed);
        IN_REALLOC[hart].store(true, Ordering::Relaxed);
        RALLOC_OLD[hart].store(old, Ordering::Relaxed);
        RALLOC_NEW[hart].store(0, Ordering::Relaxed);
    }
}

/// 退出 realloc 窗口（portal::grow 收尾；grow 失败/in-place 成功时旧块未 free，
/// 窗口在此清）。恢复 SIE（begin 关过才恢复）。
#[cfg(debug_assertions)]
pub fn end_realloc() {
    let hart = crate::machine::hart_id().min(15) as usize;
    IN_REALLOC[hart].store(false, Ordering::Relaxed);
    let sie = RALLOC_SIE[hart].swap(0, Ordering::Relaxed);
    if sie != 0 {
        // SAFETY: begin_realloc 关的 SIE，成对恢复。
        unsafe { riscv::register::sstatus::set_sie() };
    }
}

/// 分配事件：活块入账（KernelHeap 整块毒化）。caller = 分配点回溯（业务调用帧
/// 返回地址，分配现场符号化用）。
/// 用户堆不 poison（键 = [`key`] 页索引编码，非地址；且用户页维持清零语义，见
/// [`OwnerKind`]）——只入 ledger 账。
#[inline]
pub fn on_alloc(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(debug_assertions)]
    {
        // site 须先于任何函数调用捕获（jalr 覆写 ra——已实证：ra 读数曾是
        // poison 返回点，全部块 site 同址失真）。
        let (site, site2) = alloc_site(4);
        // realloc 窗口：记首笔新块（grow 内第一笔 alloc；窗口关 SIE，后续分配
        // 必属同 grow 链，首笔即 allocate 新块）。
        let hart = crate::machine::hart_id().min(15) as usize;
        if IN_REALLOC[hart].load(Ordering::Relaxed)
            && RALLOC_NEW[hart].load(Ordering::Relaxed) == 0
        {
            RALLOC_NEW[hart].store(addr, Ordering::Relaxed);
        }
        if let OwnerKind::KernelHeap = kind {
            poison(addr, size);
        }
        ledger::LEDGER.mark(addr, size, site, site2, kind);
    }
}

/// 释放事件：活块注销 + KernelHeap 本体毒化复写（头 8B 随后被 freelist 头插
/// 覆盖，其余保持毒化——UAF 读数变 0xCD）。用户堆不 poison（同 [`on_alloc`]：
/// 键非地址、维持清零语义）——只注销账目。
#[inline]
pub fn on_free(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(debug_assertions)]
    {
        // 持久块释放判定（基线记录后）：基线块被 free 只有两种来源——realloc
        // 搬家（合法：portal::grow 窗口内，新块已 mark，账目迁址）或错释。
        // 窗口命中 → rehome；窗口外命中 → report panic 转储带完整调用栈
        // （第一现场——关机差集只能报「哪块没了」，这里报「谁放的」）。
        let hart = crate::machine::hart_id().min(15) as usize;
        if IN_REALLOC[hart].load(Ordering::Relaxed) && addr == RALLOC_OLD[hart].load(Ordering::Relaxed)
        {
            IN_REALLOC[hart].store(false, Ordering::Relaxed);
            crate::memory::allocator::fence::audit::rehome_baseline(
                addr,
                RALLOC_NEW[hart].load(Ordering::Relaxed),
            );
        } else if crate::memory::allocator::fence::audit::is_baseline_block(addr) {
            report(
                IntegrityViolation::UnregisteredFree,
                addr,
                format_args!("persistent baseline block freed: {addr:#x} size {size}"),
            );
        }
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

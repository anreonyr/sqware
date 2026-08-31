// 护栏层（fence）— 内存运行时不变量检查，内嵌在生产路径（in-path）
//
// 与「自测（selftest，out-of-path 验收用例）」相对：护栏是功能运行的自我证明，
// 命中即 halt（panic → crash scene）。四个成员：
//   checker  — 分配器链式不变式断言（block/frame 的 freepool 判重、越界、环与
//              流水观测）。钩子恒编译、单行调用；函数体 debug-gated。
//   banker   — 页金库占位（无锁原子位图；free 区每页 1 bit，debit/credit/is_held）。
//   ledger   — 活块账本（hashbrown 登记表；mark/unmark/verify/canary，锁内零分配）。
//   audit    — 核查侧（多源交叉核对 audit()、关机类别记账检查 check_baseline、
//              持久注册表、页清残留 page_clear、统计 stats）。
// 模块根 = banker/ledger/checker/audit 共享的处置原语：report（违例→trace→panic）、
// IntegrityViolation（违例类目）、poison（毒化标记）、OwnerKind（登记类别）、
// **所有权类别**（FrameClass/BlockClass，见下）与相关常量，以及**事件入口**
// （on_alloc/on_free/on_frame_alloc/on_frame_free）——分配器热路径对其的调用是
// 一行无 cfg 的语义事件，asm 读 ra、poison、记账全部收在本层内部。
//
// # 解耦纪律：类别机制收在本层，分配器文件零审计词汇
//
// 帧类别表（FRAME_CLASS per-page 字节表，与 banker 位图同构）、块类别（ledger
// 记录 class 字段）、类别计数、打标分配器（alloc_frame / alloc_block——ZST
// 包装委托分配器 + 返回前标注）、realloc 类别继承全部在本层实现。frame/block
// 分配器**不携带任何类别参数**：分配点经 `alloc_frame(class)` /
// `alloc_block(class)` 取打标分配器（唯一 fence 词汇入口），释放路径类别由本层
// 自存表/账本读出。debit/credit 恒发生、与类别无关——类别只影响计数维度，
// 类别错乱只失真计数（boot sanity 可抓），不破坏 banker 配对。
//
// gate 语义：**audit 是 cargo feature**（kernel/Cargo.toml `audit`，default 引入）
// ——debug 构建默认开启（任务验收命令 `cargo run -p kernel` 即带），release 可
// 显式 `--features audit` 开启、`--no-default-features` 关闭。checker 独立
// （debug-only）；banker/ledger/audit → 模块根（feature-gated）。feature 关闭时
// 打标分配器退化为裸委托（零开销）。
//
// 依赖方向（无环）：checker 独立；banker/ledger → 模块根；audit → 模块根 + banker + ledger。

// ── 所有权类别（关机审计的记账维度）──
//
// 每帧/每块按生命周期归属一个类别；类别计数（FRAME_COUNTS/BLOCK_COUNTS）由
// 打标路径（alloc_frame/alloc_block 分配后标注）与释放路径（on_frame_free 查
// FRAME_CLASS 表 / on_free 经 unmark 读出）成对维护。关机检查
// （audit::check_baseline）只对「任务类归零」做断言——合法形态演化（容器扩容、
// realloc 搬家（类别继承）、池页周转、审计工具自身分配）只是类别内部的变化，
// 不需要任何赦免机制（替代旧「boot 身份快照 vs 关机差集」的 rehome/adopt/
// 基线余量/AUDITING 豁免——见 1634c36 教训：快照物化 prime 自扰即
// mid-collection realloc → 孤儿帧）。审计工具自身的暂态分配走默认 Persistent
// 类、在检查内成对归还——新框架无差集检查，对其天然免疫（旧框架的豁免与
// drain 守卫是差集模型的遗留，已全部删除）。
//
// 帧类别存 fence 自己的 per-page 字节表（FRAME_CLASS，与 banker 位图同构）；
// 块类别存 ledger 记录（mark 默认 Persistent、relabel 改类）——分配器文件
// 零类别词汇（解耦纪律见模块头）。
//
// 编码即 ABI：0 = Persistent 为默认（表全零即「未标注」语义，Persistent 标注
// 幂等）；from_u8 对未知值防御归 Persistent。

#![allow(unused)]
use alloc::boxed::Box;
use alloc::fmt;
use core::alloc::{AllocError, Allocator};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::lock::OnceLock;

pub mod audit;
pub mod banker;
pub mod checker;
pub mod ledger;

// ── 公共处置原语（audit-feature-gated；feature 关闭时编译为空，零开销）──

/// 毒化模式字节（分配/释放填充：未初始化读、UAF 读数的现行标记）。
#[cfg(feature = "audit")]
pub const POISON: u8 = 0xCD;
/// slack canary 期望值（写进块尾 slack 8 字节；释放时核对）。
#[cfg(feature = "audit")]
pub(crate) const CANARY_MAGIC: u64 = 0x51A7_0D1E_CAFE_BEEF;
/// canary 所需最小 slack 字节数（不足则本块不设 canary）。
#[cfg(feature = "audit")]
pub(crate) const CANARY_MIN_SLACK: usize = 8;

/// 登记类别（Ledger 记录归属；用户堆不 poison/canary——维持清零语义）。
/// 放模块根（不 gate）：事件入口签名引用它，release 下 ledger 模块为空也可编译。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    KernelHeap,
    UserHeap,
}

/// 帧生命周期类别（关机审计维度；编码写 FRAME_CLASS 表，`as usize` 作计数下标）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum FrameClass {
    /// boot 持久帧（trap 栈 / spare 仓 / 内核窗口帧；未显式标注的默认类）——
    /// 持久注册表逐项核 held（②）。
    Persistent = 0,
    /// 块池 prime 借页（block.rs `prime`）——自由周转，仅诊断计数。
    Pool = 1,
    /// 页表页（table.rs `PageTable::new`：root 与 walk_mut 子表）——关机与
    /// kernel-root walk 计数核对（audit::check_baseline ③）。
    Table = 2,
    /// 任务生命周期帧（栈体 / trap 帧 / 懒页 / owned 数据帧 / COW 页）——
    /// **关机必须归零**（①）。
    Task = 3,
}

impl FrameClass {
    /// 表值还原。未知值（表损坏）防御归 Persistent——只失真计数维度（boot
    /// sanity 可抓），banker 配对不受影响（debit/credit 与类别无关）。
    pub(crate) fn from_u8(v: u8) -> FrameClass {
        match v {
            0 => FrameClass::Persistent,
            1 => FrameClass::Pool,
            2 => FrameClass::Table,
            3 => FrameClass::Task,
            _ => FrameClass::Persistent,
        }
    }
}

/// 块生命周期类别（Ledger 记录字段；`as usize` 作计数下标，编码即 ABI）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum BlockClass {
    /// 默认——boot 期容器、调度器、Team/Space 簿记。
    Persistent = 0,
    /// 任务生命周期块（Arc<Task>/Arc<TaskIdent>/闭包装箱/用户堆记录）——
    /// **关机必须归零**（①）。
    Task = 1,
}

impl BlockClass {
    /// 编码还原（realloc 窗口存 u8）。未知值防御性归 Persistent。
    pub(crate) fn from_u8(v: u8) -> BlockClass {
        match v {
            0 => BlockClass::Persistent,
            1 => BlockClass::Task,
            _ => BlockClass::Persistent,
        }
    }
}

/// 帧类别计数（标注 +1 / 释放查表 −1）。关机只断言 Task 归零，
/// 其余类别为诊断/未来检查用。
pub(crate) static FRAME_COUNTS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// 块类别计数（mark/relabel +1 / unmark −1）。
pub(crate) static BLOCK_COUNTS: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

/// 类别计数读取（audit 读侧）。
pub(crate) fn frame_count(c: FrameClass) -> usize {
    FRAME_COUNTS[c as usize].load(Ordering::Relaxed)
}
/// 类别计数读取（audit 读侧）。
pub(crate) fn block_count(c: BlockClass) -> usize {
    BLOCK_COUNTS[c as usize].load(Ordering::Relaxed)
}

// ── 帧类别表（fence 所有；分配器文件零类别词汇）──

/// 帧类别表：free 区每页 1 字节（0 = Persistent 默认），与 banker 位图同构
/// （idx = (pa − base)/PAGE_SIZE，base 同 banker.init）。init 由 banker.init
/// 同步装配（bump 后端，boot 单核）。
static FRAME_CLASS: OnceLock<Box<[AtomicU8]>> = OnceLock::new();
/// 表基址（与 banker 同源）。
static FRAME_CLASS_BASE: AtomicUsize = AtomicUsize::new(0);

/// 装配帧类别表（banker.init 同点调用；先于任何帧分配）。
#[cfg(feature = "audit")]
pub(crate) fn init_frame_class(base: usize, pages: usize) {
    let table: Box<[AtomicU8]> = (0..pages).map(|_| AtomicU8::new(0)).collect();
    FRAME_CLASS_BASE.store(base, Ordering::Relaxed);
    assert!(FRAME_CLASS.set(table).is_ok(), "frame class table double init");
}

/// 表下标（与 banker.idx 同算式）。
fn frame_class_slot(pa: usize) -> usize {
    let base = FRAME_CLASS_BASE.load(Ordering::Relaxed);
    (pa - base) / crate::memory::PAGE_SIZE
}

/// 帧类别标注（打标分配器在委托返回后调用；分配点 → fence 的唯一标注入口）。
/// 计数 +1；表项已是非 0 同类 = 幂等，异类 = 重复标注（防御 panic）。
/// Persistent 标注写 0（表全零即默认语义）。
#[cfg(feature = "audit")]
pub(crate) fn tag_frame(pa: usize, class: FrameClass) {
    let slot = frame_class_slot(pa);
    let table = FRAME_CLASS.get().expect("frame class table not initialized");
    let cur = table[slot].load(Ordering::Relaxed);
    assert!(
        cur == 0 || cur == class as u8,
        "frame {pa:#x} re-tagged: {cur} then {class:?}"
    );
    if class != FrameClass::Persistent {
        table[slot].store(class as u8, Ordering::Relaxed);
    }
    FRAME_COUNTS[class as usize].fetch_add(1, Ordering::Relaxed);
}

/// 块类别标注（打标分配器 / realloc 继承 / 用户堆记录等直接调用点）：ledger
/// 记录改类并做计数迁移（mark 已按默认 Persistent +1）。fence 对外的块标注入口。
#[cfg(feature = "audit")]
pub(crate) fn tag_block(addr: usize, class: BlockClass) {
    let old = ledger::LEDGER.relabel(addr, class);
    if old != class {
        BLOCK_COUNTS[old as usize].fetch_sub(1, Ordering::Relaxed);
        BLOCK_COUNTS[class as usize].fetch_add(1, Ordering::Relaxed);
    }
}

// ── 打标分配器（ZST；审计词汇的唯一出口——分配点经 alloc_frame/alloc_block 取）──

/// 打标帧分配器：委托 frame 分配器，返回前标注类别（表 + 计数）。释放侧类别
/// 由 on_frame_free 查表读出——frame 分配器文件不携带类别。
struct TaggedFrameAllocator(FrameClass);

unsafe impl Allocator for TaggedFrameAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let p = crate::memory::allocator::frame::allocator().allocate(layout)?;
        #[cfg(feature = "audit")]
        tag_frame(p.as_ptr().cast::<u8>() as usize, self.0);
        Ok(p)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        // SAFETY: 同 frame 分配器契约（Box drop 同源 layout）。
        unsafe { crate::memory::allocator::frame::allocator().deallocate(ptr, layout) };
    }
}

static FRAME_TAGGED: [TaggedFrameAllocator; 4] = [
    TaggedFrameAllocator(FrameClass::Persistent),
    TaggedFrameAllocator(FrameClass::Pool),
    TaggedFrameAllocator(FrameClass::Table),
    TaggedFrameAllocator(FrameClass::Task),
];

/// 取打标帧分配器（分配点入口；feature 关闭时退化为裸 frame 分配器——零开销）。
pub(crate) fn alloc_frame(class: FrameClass) -> &'static dyn Allocator {
    #[cfg(feature = "audit")]
    {
        &FRAME_TAGGED[class as usize]
    }
    #[cfg(not(feature = "audit"))]
    {
        let _ = class;
        crate::memory::allocator::frame::allocator()
    }
}

/// 打标块分配器：委托块分配器（≤ 半页块域；Task 类当前无帧级缓冲用例），返回前
/// relabel 类别（mark 已按默认 Persistent 入账）。释放侧类别由 on_free 经
/// unmark 读出——block 分配器文件不携带类别。
struct TaggedBlockAllocator(BlockClass);

unsafe impl Allocator for TaggedBlockAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        // 块域守卫：> 半页走 frame 后端（帧侧类别经 alloc_frame 标注，块侧
        // relabel 无法覆盖）——Task 类当前无帧级缓冲用例，防御性拒绝。
        if layout.size() > crate::memory::PAGE_SIZE / 2 {
            return Err(AllocError);
        }
        let p = crate::memory::allocator::block::allocator().allocate(layout)?;
        #[cfg(feature = "audit")]
        if self.0 != BlockClass::Persistent {
            tag_block(p.as_ptr().cast::<u8>() as usize, self.0);
        }
        Ok(p)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        // SAFETY: 同块分配器契约（Box drop 同源 layout）。
        unsafe { crate::memory::allocator::block::allocator().deallocate(ptr, layout) };
    }
}

static BLOCK_TAGGED: [TaggedBlockAllocator; 2] = [
    TaggedBlockAllocator(BlockClass::Persistent),
    TaggedBlockAllocator(BlockClass::Task),
];

/// 取打标块分配器（分配点入口；feature 关闭时退化为裸块分配器——零开销）。
pub(crate) fn alloc_block(class: BlockClass) -> &'static dyn Allocator {
    #[cfg(feature = "audit")]
    {
        &BLOCK_TAGGED[class as usize]
    }
    #[cfg(not(feature = "audit"))]
    {
        let _ = class;
        crate::memory::allocator::block::allocator()
    }
}

/// 完整性违例类别（report 的字段；repr(u8) 供 trace 事件编码，**顺序即 ABI**）。
#[cfg(feature = "audit")]
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
#[cfg(feature = "audit")]
pub fn poison(addr: usize, len: usize) {
    // SAFETY: 调用方保证区间独占；volatile 写。
    let p = addr as *mut u8;
    for i in 0..len {
        unsafe { p.add(i).write_volatile(POISON) };
    }
}

/// 统一处置（不返回）：trace 记 Mem(Integrity) → 现场直写 → panic
/// （halt 的 panic 处理器再转储 crash scene；panic 路径零分配）。
#[cfg(feature = "audit")]
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

// ── 事件入口（恒编译；体内 audit-feature-gated，feature 关闭时空体零开销）──
//
// 分配器热路径唯一可见的护栏痕迹：一行语义调用。asm 读 ra、poison 填充、
// ledger/banker 记账全部收在本层内部——纯功能文件（block/frame/space）不见
// 任何 cfg、编译器内联指令或审计词汇。audit-feature 构建做账，feature 关闭时空体经
// #[inline] 消除。

/// 用户堆活块账键（**跨模块硬不变量：键必须单射**）。
///
/// `(asid << 44) | (va >> 12)`：asid < 2^16，va < 2^56（任何受支持模式的用户
/// 半区）→ 页索引 < 2^44，两段不重叠。替代旧 `asid<<32|va`（VA≥2^32 时碰撞）。
///
/// 恒编译（release 返回 0 空体，与事件入口同惯例）。
#[inline]
pub(crate) fn key(asid: usize, va: usize) -> usize {
    #[cfg(feature = "audit")]
    {
        (asid << 44) | (va >> 12)
    }
    #[cfg(not(feature = "audit"))]
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
    /// 代码段尾（link.ld `_text_end`，.trampoline 之后）：候选 ra 须落在其下
    /// ——.data/.bss 的静态符号地址（LEDGER/FRAME_ALLOCATOR 等）与 .rodata 上的
    /// vtable/常量指针是栈上常见数据，误收即 site 失真。旧守卫用 _rodata_start
    /// （.rodata 在镜像尾）会放 .data/.bss 进来——已实证 site 全失真。
    static _text_end: u8;
}

/// .text 段上界（.trampoline 之后）。
fn text_end() -> usize {
    // SAFETY: 链接脚本符号，恒存在。
    unsafe { (&raw const _text_end).addr() }
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
#[cfg(feature = "audit")]
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
/// 新块**继承旧块类别**（[`BlockClass`]：任务类缓冲 grow 搬家后仍是任务类——
/// 关机类别归零检查才不会因搬家而漏报/误报）。仅旧块类别非 Persistent 时设窗
/// （默认类 grow 继承无意义，零开销）。
///
/// 窗口内**关 SIE**（临界区微秒级）：S-timer 抢占会让同 hart 其它任务在窗口内
/// 分配——RALLOC_NEW 被覆盖即错配继承（旧基线 rehome 同源教训：抢占污染配对
/// 已实证）。关中断后窗口内分配必属本 grow。
static IN_REALLOC: [AtomicBool; 16] = [const { AtomicBool::new(false) }; 16];
static RALLOC_OLD: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
static RALLOC_NEW: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
/// 待继承类别（begin 从 ledger 读出旧块类别；窗口内首笔新块 mark 时采用）。
static RALLOC_CLASS: [AtomicU8; 16] = [const { AtomicU8::new(0) }; 16];
/// SIE 恢复标记（0 = 无需恢复；非 0 = begin 关过 SIE，end 恢复）。
static RALLOC_SIE: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];

/// 进入 realloc 窗口（portal::grow 调用；须与 [`end_realloc`] 配对）。
/// 旧块类别非 Persistent 才设窗——其 grow 的新块继承类别（默认类无继承语义）。
/// 恒编译（体内 audit-feature-gated，与事件入口同惯例——feature 关闭时空体零开销）。
pub fn begin_realloc(old: usize) {
    #[cfg(feature = "audit")]
    {
        let hart = crate::machine::hart_id().min(15) as usize;
        if let Some(class) = ledger::LEDGER.class_of(old) {
            if class != BlockClass::Persistent {
                // 关 SIE：窗口内不可抢占（见 IN_REALLOC 注释——抢占污染配对）。
                let sie = riscv::register::sstatus::read().sie() as usize;
                // SAFETY: 关/开 SIE 仅改本 hart 中断使能位，窗口极短；end_realloc 恢复。
                unsafe { riscv::register::sstatus::clear_sie() };
                RALLOC_SIE[hart].store(sie, Ordering::Relaxed);
                IN_REALLOC[hart].store(true, Ordering::Relaxed);
                RALLOC_OLD[hart].store(old, Ordering::Relaxed);
                RALLOC_NEW[hart].store(0, Ordering::Relaxed);
                RALLOC_CLASS[hart].store(class as u8, Ordering::Relaxed);
            }
        }
    }
}

/// 退出 realloc 窗口（portal::grow 收尾；grow 失败/in-place 成功时旧块未 free，
/// 窗口在此清）。恢复 SIE（begin 关过才恢复）。恒编译（同 [`begin_realloc`]）。
pub fn end_realloc() {
    #[cfg(feature = "audit")]
    {
        let hart = crate::machine::hart_id().min(15) as usize;
        IN_REALLOC[hart].store(false, Ordering::Relaxed);
        let sie = RALLOC_SIE[hart].swap(0, Ordering::Relaxed);
        if sie != 0 {
            // SAFETY: begin_realloc 关的 SIE，成对恢复。
            unsafe { riscv::register::sstatus::set_sie() };
        }
    }
}

/// 分配事件：活块入账（KernelHeap 整块毒化）。caller = 分配点回溯（业务调用帧
/// 返回地址，分配现场符号化用）。
/// 用户堆不 poison（键 = [`key`] 页索引编码，非地址；且用户页维持清零语义，见
/// [`OwnerKind`]）——只入 ledger 账。
/// 类别：mark 按默认 Persistent 入账并计数；realloc 窗口内首笔新块经
/// [`relabel_block`] 继承旧块类别（计数迁移）——打标分配器的显式类别在委托
/// 返回后走同一 relabel 路径。
#[inline]
pub fn on_alloc(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(feature = "audit")]
    {
        // site 须先于任何函数调用捕获（jalr 覆写 ra——已实证：ra 读数曾是
        // poison 返回点，全部块 site 同址失真）。
        let (site, site2) = alloc_site(4);
        // realloc 窗口：记首笔新块（grow 内第一笔 alloc；窗口关 SIE，后续分配
        // 必属同 grow 链，首笔即 allocate 新块）并继承旧块类别（begin_realloc
        // 已从 ledger 读出存入 RALLOC_CLASS）。
        let hart = crate::machine::hart_id().min(15) as usize;
        let first_new = IN_REALLOC[hart].load(Ordering::Relaxed)
            && RALLOC_NEW[hart].load(Ordering::Relaxed) == 0;
        if first_new {
            RALLOC_NEW[hart].store(addr, Ordering::Relaxed);
        }
        if let OwnerKind::KernelHeap = kind {
            poison(addr, size);
        }
        ledger::LEDGER.mark(addr, size, site, site2, kind, BlockClass::Persistent);
        BLOCK_COUNTS[BlockClass::Persistent as usize].fetch_add(1, Ordering::Relaxed);
        if first_new {
            let inherited = BlockClass::from_u8(RALLOC_CLASS[hart].load(Ordering::Relaxed));
            if inherited != BlockClass::Persistent {
                tag_block(addr, inherited);
            }
        }
    }
}

/// 释放事件：活块注销 + KernelHeap 本体毒化复写（头 8B 随后被 freelist 头插
/// 覆盖，其余保持毒化——UAF 读数变 0xCD）。用户堆不 poison（同 [`on_alloc`]：
/// 键非地址、维持清零语义）——只注销账目。
/// 类别自 ledger 记录读出（mark/relabel 时定型）：注销后按类减计数——realloc
/// 搬家旧块同样走本路径（新块已在窗口内继承类别，账目平衡）。
#[inline]
pub fn on_free(addr: usize, size: usize, kind: OwnerKind) {
    #[cfg(feature = "audit")]
    {
        let class = ledger::LEDGER.unmark(addr, size);
        BLOCK_COUNTS[class as usize].fetch_sub(1, Ordering::Relaxed);
        if let OwnerKind::KernelHeap = kind {
            poison(addr, size);
        }
    }
}

/// 帧分配事件：页金库取出（Free→held；双取出 / 活堆页泄漏进池现行）。
/// debit 恒发生、与类别无关——类别标注与计数由打标分配器（[`alloc_frame`]）在
/// 委托返回后经 [`tag_frame`] 完成（frame 分配器文件零类别词汇）。
#[inline]
pub fn on_frame_alloc(addr: usize) {
    #[cfg(feature = "audit")]
    {
        banker::BANKER.debit(addr);
    }
}

/// 帧释放事件：页金库存入（held→Free；存入陌生页 / 双释放现行）。
/// 类别自 FRAME_CLASS 表读出（标注时写入）——按类减计数后清表项；credit 恒
/// 发生、与类别无关（banker 配对不受类别错乱影响）。
#[inline]
pub fn on_frame_free(addr: usize) {
    #[cfg(feature = "audit")]
    {
        let slot = frame_class_slot(addr);
        let table = FRAME_CLASS.get().expect("frame class table not initialized");
        let class = FrameClass::from_u8(table[slot].load(Ordering::Relaxed));
        table[slot].store(0, Ordering::Relaxed);
        FRAME_COUNTS[class as usize].fetch_sub(1, Ordering::Relaxed);
        banker::BANKER.credit(addr);
    }
}

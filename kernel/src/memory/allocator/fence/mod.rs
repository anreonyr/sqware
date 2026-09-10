// 护栏层（fence）— 内存运行时不变量检查，内嵌在生产路径（in-path）
//
// **「in-path」是结构意义上的**：事件入口恒编译、调用点就在分配热路径上；但护栏本体
// 只在验收档里存在（`audit` feature / `debug_assertions`），产品档（default）里那些调用
// 是空体——实测三档产物：default 一条护栏串都没有，audit 带 banker/ledger，harden 带
// checker/lockdep，**没有任何一档同时带两半**。这个事实记在 docs §10.16。
//
// 与「自测（selftest，out-of-path 验收用例）」相对：护栏是功能运行的自我证明，
// 命中即 halt（panic → crash scene）。五个成员：
//   kind     — **记账的唯一维度**：分配对象种类与它自带的期望终值 / 策略 / 键域。
//   checker  — 分配器链式不变式断言（block/frame 的 freepool 判重、越界、环与
//              流水观测）。钩子恒编译、单行调用；函数体 debug-gated。
//   banker   — 页金库占位（无锁原子位图；free 区每页 1 bit，debit/credit/is_held）。
//   ledger   — 活块账本（hashbrown 登记表；mark/unmark/relabel/canary，锁内零分配）。
//   audit    — 核查侧（多源交叉核对 audit()、关机**逐种类终值**检查 check_baseline、
//              持久注册表、页清残留 page_clear、站点/名册探针）。
// 模块根 = kind/banker/ledger/checker/audit 共享的处置原语：report（违例→trace→panic）、
// IntegrityViolation（违例类目）、poison（毒化标记）、帧种类表与相关常量，以及**事件入口**
// （on_alloc/on_free/on_frame_alloc/on_frame_free）——分配器热路径对其的调用是
// 一行无 cfg 的语义事件，asm 读 ra、poison、记账全部收在本层内部。
//
// # 解耦纪律：种类机制收在本层，分配器文件零种类词汇
//
// 帧种类表（FRAME_KIND per-page 字节表，与 banker 位图同构）、块种类（ledger
// 记录 kind 字段）、种类计数（statistics）、打标分配器（alloc_frame / alloc_block——
// ZST 包装委托分配器 + 返回前标注）全部在本层实现。**分配器不携带种类参数**：
// 分配点经 `tag!` 装饰器 / `tagged_alloc(kind)`（唯一 fence 词汇入口）标注，
// 释放路径的种类由本层自存表/账本读出。debit/credit 恒发生、与种类无关——种类只影响
// 计数维度，种类错乱只失真计数（`tag` 的 side 断言会当场抓住），不破坏 banker 配对。
//
// 帧的种类**由造对象的那一层标注**：`SpaceInner::frame()` 只领一帧、不认识种类，
// claim/materialize 的帧来源由调用者以闭包给出（`attach` 早就是这个形状）。
//
// gate 语义：**audit 是 cargo feature**（kernel/Cargo.toml `audit`，**非默认**，
// 需显式 `--features audit` 开启；与 debug_assertions 无关，release 也能跑审计）。
// checker 独立（debug-only）；banker/ledger/audit → 模块根（feature-gated）。
// feature 关闭时装饰器（tag!）退化为裸表达式（零开销），ledger 模块整体 cfg out，
// scene 中 sweep_canaries 等调用也必须同步 gate 在 audit feature 下。
//
// 依赖方向（无环）：kind 独立（恒编译）；checker 独立；banker/ledger → 模块根；
// audit → 模块根 + banker + ledger。

// ── 对象种类（关机记账的唯一维度）──
//
// 每帧/每块按对象种类记账（[`Kind`]），种类自带**期望终值**（`End`）与策略。
// 关机检查（audit::check_baseline）按终值分组：任务侧对象归零（真泄漏判据）、
// 表页数与内核根表 walk 相符、持久对象逐个仍在手、周转/未标注只报数。
// **没有赦免机制**（替代旧「boot 身份快照 vs 关机差集」的 rehome/adopt/基线余量/
// AUDITING 豁免——见 1634c36 教训：快照物化 prime 自扰即 mid-collection realloc →
// 孤儿帧）。审计工具自身的暂态分配走 `Plain` 类、在检查内成对归还。
//
// 帧种类存 fence 自己的 per-page 字节表（FRAME_KIND，与 banker 位图同构）；
// 块种类存 ledger 记录（mark 默认 Plain、relabel 改种类）——分配器文件零种类词汇。

#![allow(unused)]
use alloc::boxed::Box;
use alloc::fmt;
use core::alloc::{AllocError, Allocator};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::lock::OnceLock;
use crate::runtime::diagnose::frame::StackReader;

pub mod audit;
pub mod banker;
pub mod checker;
pub mod kind;
pub mod ledger;

#[cfg(feature = "audit")]
pub(crate) use kind::{End, Keys, Side};
pub(crate) use kind::{KIND_COUNT, Kind};

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

// ── 帧种类表（fence 所有；分配器文件零种类词汇）──

/// 帧种类表：free 区每页 1 字节（0 = Plain 未标注），与 banker 位图同构
/// （idx = (pa − base)/PAGE_SIZE，base 同 banker.init）。init 由 banker.init
/// 同步装配（bump 后端，boot 单核）。
static FRAME_KIND: OnceLock<Box<[AtomicU8]>> = OnceLock::new();
/// 表基址（与 banker 同源）。
static FRAME_KIND_BASE: AtomicUsize = AtomicUsize::new(0);

/// 装配帧种类表（banker.init 同点调用；先于任何帧分配）。
#[cfg(feature = "audit")]
pub(crate) fn init_frame_kind(base: usize, pages: usize) {
    let table: Box<[AtomicU8]> = (0..pages).map(|_| AtomicU8::new(0)).collect();
    FRAME_KIND_BASE.store(base, Ordering::Relaxed);
    assert!(
        FRAME_KIND.set(table).is_ok(),
        "frame kind table double init"
    );
}

/// 表下标（与 banker.idx 同算式）。
fn frame_kind_slot(pa: usize) -> usize {
    let base = FRAME_KIND_BASE.load(Ordering::Relaxed);
    (pa - base) / crate::memory::PAGE_SIZE
}

/// 标注（tag）：把对象种类记入归属账。**种类自带它该去哪张表**（[`Kind::side`]）——
/// 今天这一步是按「在不在账本里」**猜**的，猜错就把块内地址写进帧种类表（已实证的
/// 污染事故：块级 Arc 误用数据指针）；有了 side 就变成**核对**，不符当场报违例。
///
/// 未标注（`Plain`）仍按老规矩分流（账本有账 → 改账，否则 → 帧表）。
/// 帧侧：`Plain` 写 0（表全零即默认语义），表项同类幂等、异类重复标注报违例。
/// 种类计数由 statistics 维护（见 record_frame_relabel / record_block_relabel）。
/// audit-feature-gated：audit 关闭时 ledger / FRAME_KIND 整体 cfg out,
/// tag 调用方由装饰器 tag! 宏展开——audit 关闭时该宏退化为裸表达式,不调 tag。
#[cfg(feature = "audit")]
pub(crate) fn tag(addr: usize, kind: Kind) {
    match kind.side() {
        // 账本侧对象：账本里必须有它（没有 = mark 没走 / 标错了对象）。
        Some(Side::Ledger) => {
            let old = ledger::LEDGER.relabel(addr, kind);
            if old != kind {
                crate::memory::allocator::statistics::record_block_relabel(old, kind);
            }
        }
        // 帧侧对象：账本里不该有它。
        Some(Side::Frame) => {
            if ledger::LEDGER.kind_of(addr).is_some() {
                report(
                    IntegrityViolation::MisplacedKind,
                    addr,
                    format_args!("frame kind {kind:?} tagged on a ledger block"),
                );
            }
            let slot = frame_kind_slot(addr);
            let table = FRAME_KIND.get().expect("frame kind table not initialized");
            match Kind::from_u8(table[slot].load(Ordering::Relaxed)) {
                Kind::Plain => {
                    table[slot].store(kind as u8, Ordering::Relaxed);
                    crate::memory::allocator::statistics::record_frame_relabel(Kind::Plain, kind);
                }
                cur if cur == kind => {} // 同类幂等
                cur => report(
                    IntegrityViolation::MisplacedKind,
                    addr,
                    format_args!("frame re-tagged: {cur:?} then {kind:?}"),
                ),
            }
        }
        // 未标注（`None` 只有 `Plain` 这一个值）：什么都不用做——帧表里 0 就是它、
        // 账上 mark 默认也是它。声明"我不知道这是什么"不产生任何记账动作。
        None => {}
    }
}

/// 读帧种类（只读；未标记载的页读出 `Plain`）。审计侧核"登记声明的种类 == 表里的种类"。
#[cfg(feature = "audit")]
pub(crate) fn frame_kind(pa: usize) -> Kind {
    let slot = frame_kind_slot(pa);
    let table = FRAME_KIND.get().expect("frame kind table not initialized");
    Kind::from_u8(table[slot].load(Ordering::Relaxed))
}

/// 摘标（untag——tag 的对偶）：帧释放路径读种类并清表项,按类减计数
/// （statistics::record_frame_give）。块侧对偶 = ledger 的 mark/unmark
/// （unmark 返回种类）。
fn untag_frame(addr: usize) -> Kind {
    #[cfg(feature = "audit")]
    {
        let slot = frame_kind_slot(addr);
        let table = FRAME_KIND.get().expect("frame kind table not initialized");
        let kind = Kind::from_u8(table[slot].load(Ordering::Relaxed));
        table[slot].store(Kind::Plain as u8, Ordering::Relaxed);
        crate::memory::allocator::statistics::record_frame_give(kind);
        kind
    }
    #[cfg(not(feature = "audit"))]
    {
        let _ = addr;
        Kind::Plain
    }
}

// ── 装饰器（tag!——tag 原语的宏形式；audit 关闭时退化为裸表达式，零开销）──

/// 装饰器的取址协议：分配结果指针 = 分配基址的类型实现（Box/NonNull——
/// `.as_ptr()` 即基址）。泛型恒等函数（[`tagged`]）经本 trait 取地址：期望类型
/// 自返回位传入参数表达式（`Box::try_new_zeroed_in` 的 T 由此推断），比宏内
/// let 绑定保留类型流。**None = 无实际分配**（ZST 装箱的悬垂对齐指针，如空
/// 闭包 `|| {}` 的 Box——不分配即无需标注）。
///
/// **块级 Arc 禁用本协议**（ledger 键 = 精确块基址，`Arc::as_ptr` 是数据指针
/// ≠ 基址，头布局是内部实现）——块级 Arc 经 [`tagged_alloc`] 在分配器侧标注
/// （allocate 返回基址）。已实证：块级 Arc 误用数据指针会把块内地址当帧写入
/// FRAME_KIND 表（污染页条目）——今天 `tag` 的 side 断言会当场抓住。
/// 帧级 Arc（COW 共享帧）可用：数据指针落在
/// 同页内，FRAME_KIND 按页分槽、取址容差一页。
pub trait AsAllocAddr {
    /// 分配基址（指针裸转丢弃元数据）；None = 无实际分配（ZST 悬垂指针）。
    fn alloc_addr(&self) -> Option<usize>;
}

/// 地址是否落在 free 区内（分配基址判定；ZST 悬垂指针如 0x1 在此之外）。
fn in_free_region(addr: usize) -> bool {
    let base = FRAME_KIND_BASE.load(Ordering::Relaxed);
    addr >= base && addr < crate::machine::dram_edge().unwrap_or(0x9000_0000)
}

impl<T: ?Sized, A: Allocator> AsAllocAddr for Box<T, A> {
    fn alloc_addr(&self) -> Option<usize> {
        let p = Box::as_ptr(self) as *const u8 as usize;
        in_free_region(p).then_some(p)
    }
}

impl AsAllocAddr for NonNull<[u8]> {
    fn alloc_addr(&self) -> Option<usize> {
        Some(self.as_ptr() as *const u8 as usize)
    }
}

impl<T: ?Sized, A: Allocator> AsAllocAddr for alloc::sync::Arc<T, A> {
    fn alloc_addr(&self) -> Option<usize> {
        let p = alloc::sync::Arc::as_ptr(self) as *const u8 as usize;
        in_free_region(p).then_some(p)
    }
}

/// 标注的恒等函数（装饰器本体）：按种类标注 `v` 的分配基址后原值返回。
/// 泛型参数 V 自返回位推断——期望类型穿过参数表达式（推断友好）。
#[cfg(feature = "audit")]
pub fn tagged<V: AsAllocAddr>(kind: Kind, v: V) -> V {
    if let Some(addr) = v.alloc_addr() {
        tag(addr, kind);
    }
    v
}

/// 标注块分配器（Arc::new_in 的分配器参数；与装饰器同词族，audit 关闭时退化
/// 为裸块分配器——零开销）。allocate 返回块基址、relabel 在基址上发生——Arc
/// 数据指针 ≠ 基址，装饰器无法覆盖（见 [`AsAllocAddr`]）。
#[cfg(feature = "audit")]
struct TaggedBlockAlloc(Kind);

#[cfg(feature = "audit")]
unsafe impl Allocator for TaggedBlockAlloc {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let p = crate::memory::allocator::block::allocator().allocate(layout)?;
        if self.0 != Kind::Plain {
            tag(p.as_ptr().cast::<u8>() as usize, self.0);
        }
        Ok(p)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: core::alloc::Layout) {
        // SAFETY: 同块分配器契约（Box drop 同源 layout）。
        unsafe { crate::memory::allocator::block::allocator().deallocate(ptr, layout) };
    }
}

/// 打标块分配器表：按种类编码索引（`Plain` / `Task` 各一个；其余种类走
/// `tagged_alloc` 的动态分支——见下）。
#[cfg(feature = "audit")]
static TAGGED_BLOCK: [TaggedBlockAlloc; 2] =
    [TaggedBlockAlloc(Kind::Plain), TaggedBlockAlloc(Kind::Task)];

/// 取标注块分配器（Arc 类块的标注入口；feature 关闭时退化为裸块分配器）。
/// 目前只有 `Plain` 与 `Task` 两个常驻实例——其余种类经 [`tagged`] 在分配后标注。
pub(crate) fn tagged_alloc(kind: Kind) -> &'static dyn Allocator {
    #[cfg(feature = "audit")]
    {
        match kind {
            Kind::Plain => &TAGGED_BLOCK[0],
            Kind::Task => &TAGGED_BLOCK[1],
            other => {
                let _ = other;
                crate::memory::allocator::block::allocator()
            }
        }
    }
    #[cfg(not(feature = "audit"))]
    {
        let _ = kind;
        crate::memory::allocator::block::allocator()
    }
}

/// 装饰器：`tag!(Trap, e)`——audit 开启时展开为 `tagged(Kind::Trap, e)`；
/// audit 关闭时展开为裸 `e`（零开销）。分配在交付前即已标注（求值未交付，
/// 抢占安全——标注对象恒为本次分配）；摘标（untag）由释放路径自动完成，
/// 装饰点无需配对代码。
#[macro_export]
macro_rules! tag {
    ($kind:ident, $e:expr) => {{
        #[cfg(feature = "audit")]
        {
            $crate::memory::allocator::fence::tagged(
                $crate::memory::allocator::fence::Kind::$kind,
                $e,
            )
        }
        #[cfg(not(feature = "audit"))]
        {
            $e
        }
    }};
}

/// 完整性违例类别（report 的字段；repr(u8) 供 trace 事件编码，**顺序即 ABI**：
/// 新增只许追加在末尾）。
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
    /// 多源交叉核对不一致（audit / 页清残留 / 关机终值）。
    AuditDivergence = 9,
    /// 种类与它落的表不符（帧种类标在账本块上 / 同址重复标注成两种 / 账本种类无账）。
    MisplacedKind = 10,
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

/// 空间死亡事件：注销该 asid 名下全部用户堆账（与 [`on_free`] 同域的销账入口，
/// 只是批量、由**空间**而非任务触发——见 `ledger::Ledger::retire`）。
///
/// 调用点：`Space::drop`，须先于 ASID 归还（asid 一旦复用，键即换主）。
#[inline]
pub(crate) fn retire(asid: usize) {
    #[cfg(feature = "audit")]
    {
        let n = ledger::LEDGER.retire(asid);
        if n > 0 {
            crate::putln!("[audit] space asid {asid} retired {n} user-heap records");
        }
    }
    #[cfg(not(feature = "audit"))]
    {
        let _ = asid;
    }
}

/// 分配点回溯（诊断 site）：从当前 fp 沿标准 RV64 帧链上溯 depth 帧，取返回地址。
///
/// debug O0 调用链稳定：帧 0 = block 分配器（on_alloc 的调用者）、再上是
/// core::alloc 与装箱/容器/业务帧。core 预编译库（__rust_alloc 等）无 FP
/// （与 diagnose::scene 回溯同款现象），链在彼处即断——断点前最后有效 ra 即 site。
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
/// 预编译库处断（无 FP，与 diagnose::scene 回溯同款问题）后，从断点帧顶向上扫描
/// 收集候选 ra（4 对齐 + 镜像 .text + 非重复；连续 SCAN_GAP 字无候选 = 已越
/// 活跃帧区，停）。返回 site = 扫描候选第 5 个（前 4 个 ≈ core/分配器帧，
/// 无区分度）：候选序列 ≈ [fmt::num、block、block、hybrid、业务帧…]，取业务帧
/// 返回地址供离线 addr2line。
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
fn alloc_site(depth: usize) -> usize {
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
    // 采样经 frame::StackReader（领域无关采样通道：walk_raw 翻译 + DRAM 守卫 +
    // 页缓存 + unaligned 读）——与 scene 回溯同源底层，不重复实现。
    let mut reader = StackReader::new(riscv::register::satp::read().bits() & ((1usize << 44) - 1));
    let mut a = fp;
    let mut gap = 0usize;
    let mut cands: [usize; 8] = [0; 8];
    let mut n = 0usize;
    while a - fp < 0x4000 && gap < 96 && n < 8 {
        let Some(w) = reader.word(a) else {
            break; // 未映射/非 R 页：越 slot 顶，停扫（不读不崩）
        };
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
    // site = 候选第 5 个（跳过 4 层 core/分配器帧——实证候选序 ≈
    // [fmt::num, block:345, block:632, hybrid, 业务帧…]）；不足则回退链尾 ra。
    if n >= 5 { cands[4] } else { ra }
}

/// realloc 窗口（per-hart）：portal::grow 显式 begin/end 标记。grow 默认路径 =
/// 同 hart「allocate 新 → copy → deallocate 旧」——窗口内 on_alloc 记首笔新块、
/// 新块**继承旧块种类**（：任务类缓冲 grow 搬家后仍是任务类——
/// 关机归零检查才不会因搬家而漏报/误报）。仅旧块种类非 Plain 时设窗
/// （默认种类 grow 继承无意义，零开销）。
///
/// 窗口内**关 SIE**（临界区微秒级）：S-timer 抢占会让同 hart 其它任务在窗口内
/// 分配——RALLOC_NEW 被覆盖即错配继承（旧基线 rehome 同源教训：抢占污染配对
/// 已实证）。关中断后窗口内分配必属本 grow。
static IN_REALLOC: [AtomicBool; 16] = [const { AtomicBool::new(false) }; 16];
static RALLOC_NEW: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
/// 待继承种类（begin 从 ledger 读出旧块种类；窗口内首笔新块 mark 时采用）。
static RALLOC_KIND: [AtomicU8; 16] = [const { AtomicU8::new(0) }; 16];
/// SIE 恢复标记（0 = 无需恢复；非 0 = begin 关过 SIE，end 恢复）。
static RALLOC_SIE: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];

/// 进入 realloc 窗口（portal::grow 调用；须与 [`end_realloc`] 配对）。
/// 旧块种类非 Plain 才设窗——其 grow 的新块继承种类（默认种类无继承语义）。
/// 恒编译（体内 audit-feature-gated，与事件入口同惯例——feature 关闭时空体零开销）。
pub fn begin_realloc(old: usize) {
    #[cfg(feature = "audit")]
    {
        let hart = crate::machine::hart_id().min(15);
        if let Some(kind) = ledger::LEDGER.kind_of(old)
            && kind != Kind::Plain
        {
            // 关 SIE：窗口内不可抢占（见 IN_REALLOC 注释——抢占污染配对）。
            let sie = riscv::register::sstatus::read().sie() as usize;
            // SAFETY: 关/开 SIE 仅改本 hart 中断使能位，窗口极短；end_realloc 恢复。
            unsafe { riscv::register::sstatus::clear_sie() };
            RALLOC_SIE[hart].store(sie, Ordering::Relaxed);
            IN_REALLOC[hart].store(true, Ordering::Relaxed);
            RALLOC_NEW[hart].store(0, Ordering::Relaxed);
            RALLOC_KIND[hart].store(kind as u8, Ordering::Relaxed);
        }
    }
}

/// 退出 realloc 窗口（portal::grow 收尾；grow 失败/in-place 成功时旧块未 free，
/// 窗口在此清）。恢复 SIE（begin 关过才恢复）。恒编译（同 [`begin_realloc`]）。
pub fn end_realloc() {
    #[cfg(feature = "audit")]
    {
        let hart = crate::machine::hart_id().min(15);
        IN_REALLOC[hart].store(false, Ordering::Relaxed);
        let sie = RALLOC_SIE[hart].swap(0, Ordering::Relaxed);
        if sie != 0 {
            // SAFETY: begin_realloc 关的 SIE，成对恢复。
            unsafe { riscv::register::sstatus::set_sie() };
        }
    }
}

/// 分配事件：活块入账（`kind.poison()` 者整块毒化）。caller = 分配点回溯（业务调用帧
/// 返回地址，分配现场符号化用）。
/// 用户堆不 poison（键 = [`key`] 页索引编码，非地址；且用户页维持清零语义，见
/// [`Kind::poison`]）——只入 ledger 账。
/// 种类：mark 按调用者说的种类入账并计数（内核堆块传 `Plain`——装饰器随后
/// `tag` 成真种类，计数迁移成对；用户堆传 `UserHeap`——它没有第二个标注点）；
/// realloc 窗口内首笔新块经 [`tag`] 继承旧块种类。
#[inline]
pub fn on_alloc(addr: usize, size: usize, kind: Kind) {
    #[cfg(feature = "audit")]
    {
        // site 须先于任何函数调用捕获（jalr 覆写 ra——已实证：ra 读数曾是
        // poison 返回点，全部块 site 同址失真）。
        let site = alloc_site(4);
        // realloc 窗口：记首笔新块（grow 内第一笔 alloc；窗口关 SIE，后续分配
        // 必属同 grow 链，首笔即 allocate 新块）并继承旧块种类（begin_realloc
        // 已从 ledger 读出存入 RALLOC_KIND）。
        let hart = crate::machine::hart_id().min(15);
        let first_new = IN_REALLOC[hart].load(Ordering::Relaxed)
            && RALLOC_NEW[hart].load(Ordering::Relaxed) == 0;
        if first_new {
            RALLOC_NEW[hart].store(addr, Ordering::Relaxed);
        }
        if kind.poison() {
            poison(addr, size);
        }
        ledger::LEDGER.mark(addr, size, site, kind);
        crate::memory::allocator::statistics::record_block_take(kind);
        if first_new {
            let inherited = Kind::from_u8(RALLOC_KIND[hart].load(Ordering::Relaxed));
            if inherited != Kind::Plain {
                tag(addr, inherited);
            }
        }
    }
}

/// 释放事件：活块注销 + `kind.poison()` 者本体毒化复写（头 8B 随后被 freelist 头插
/// 覆盖，其余保持毒化——UAF 读数变 0xCD）。用户堆不 poison（同 [`on_alloc`]：
/// 键非地址、维持清零语义）——只注销账目。
/// 种类自 ledger 记录读出（mark/relabel 时定型）：注销后按种类减计数——realloc
/// 搬家旧块同样走本路径（新块已在窗口内继承种类，账目平衡）。
/// 传入的 `kind` 只用于毒化决策，须与账上种类一致（不一致即
/// [`IntegrityViolation::MisplacedKind`]——今天传错无人发现）。
#[inline]
pub fn on_free(addr: usize, size: usize, kind: Kind) {
    #[cfg(feature = "audit")]
    {
        let rec = ledger::LEDGER.unmark(addr, size);
        crate::memory::allocator::statistics::record_block_give(rec);
        if kind != Kind::Plain && kind != rec {
            report(
                IntegrityViolation::MisplacedKind,
                addr,
                format_args!("free: said {kind:?}, ledger says {rec:?}"),
            );
        }
        if kind.poison() {
            poison(addr, size);
        }
    }
}

/// 帧分配事件：页金库取出（Free→held；双取出 / 活堆页泄漏进池现行）。
/// debit 恒发生、与种类无关——种类标注由装饰器（[`tag!`]）在求值返回后
/// 经 [`tag`] 完成（frame 分配器文件零种类词汇）。
#[inline]
pub fn on_frame_alloc(addr: usize) {
    #[cfg(feature = "audit")]
    {
        banker::BANKER.debit(addr);
    }
}

/// 帧释放事件：页金库存入（held→Free；存入陌生页 / 双释放现行）。
/// 摘标（untag——tag 的对偶）：自 FRAME_KIND 表读出种类、清表项,按种类减
/// 计数由 [`untag_frame`] 内部 record_frame_give 完成；credit 恒发生、
/// 与种类无关（banker 配对不受种类错乱影响）。
#[inline]
pub fn on_frame_free(addr: usize) {
    #[cfg(feature = "audit")]
    {
        let _kind = untag_frame(addr);
        banker::BANKER.credit(addr);
    }
    #[cfg(not(feature = "audit"))]
    {
        crate::memory::allocator::statistics::record_frame_give(Kind::Plain);
    }
}

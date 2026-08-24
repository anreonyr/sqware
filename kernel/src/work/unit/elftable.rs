//! elftable — 从 ELF 的 .symtab+.strtab 读出的符号表（B.3 符号化）。
//!
//! 职责：给定 .symtab（Elf64_Sym 数组）+ .strtab（字符串表）两个切片，产出可按
//! 地址二分查询的符号表。名字是从 strtab 里切出的 &'static str（零拷贝）；表本身
//! 一次建成后 Box::leak（boot/装载时，永不回收）。
//!
//! 两域同一坐标：符号 st_value 是虚拟地址，查询 key（sepc/stval/回溯）也是虚拟
//! 地址，故 Entry.addr: VirtAddr、lookup(a: VirtAddr)。内核恒等映射使 VA==PA 只是
//! 凑巧，语义仍是 VA。
//!
//! 存储：内核关键入口表（编译期闭合，见 [`kernel_table`]）独立挂本模块的
//! `OnceLock`——表内容全是链接期常量，生命周期不绑 KERNEL_TEAM：崩溃路径
//! （unit::init 自身失败、团队尚未注入的早期 panic）也能符号化，无需触碰团队
//! 单例。用户/团队表仍挂 `Team.elftable`。resolve(addr, team) 按地址域选表查询
//! （内核高半区/镜像恒等区 → 内核表，用户区 → 运行 team 表）。

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::lock::OnceLock;
use crate::memory::manager::addr::VirtAddr;

/// 一条符号（STT_FUNC；表内按 addr 升序）。
pub struct Entry {
    pub addr: VirtAddr,
    pub name: &'static str,
}

/// 符号表 — 有序 Entry 切片。
///
/// `entries` 是不可变共享切片（'static，`Box::leak` 建成），`Copy` 语义天然
/// 成立——内核表单例（[`kernel_table`]）经 `OnceLock<ElfTable>` 按值缓存/返回
/// 依赖它。
#[derive(Clone, Copy)]
pub struct ElfTable {
    entries: &'static [Entry],
}

// ELF64 符号表布局（Elf64_Sym，24 B/条；按字节 + LE 读取，同 parser.rs 风格）。
const SYM_SIZE: usize = 24;
const SYM_NAME: usize = 0;
const SYM_INFO: usize = 4;
const SYM_VALUE: usize = 8;
/// st_info 低 4 位 = 符号类型。
const ST_INFO_TYPE: u8 = 0x0f;
/// STT_FUNC = 函数符号。
const STT_FUNC: u8 = 2;
/// 符号无名字时的占位（'static 字面量）。
const NO_NAME: &str = "<no name>";

fn u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64(b: &[u8], off: usize) -> u64 {
    let g = |i: usize| b[off + i];
    u64::from_le_bytes([g(0), g(1), g(2), g(3), g(4), g(5), g(6), g(7)])
}

/// 从 strtab 切出 NUL 结尾的名字（越界/非 UTF-8 取占位；'static 零拷贝）。
fn name_at(strtab: &'static [u8], off: usize) -> &'static str {
    let ok = off.min(strtab.len());
    let rest = &strtab[ok..];
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    core::str::from_utf8(&rest[..end]).unwrap_or(NO_NAME)
}

/// name 防御判定（[`ElfTable::lookup`] 与现场体检 [`ElfTable::check_integrity`]
/// 共用）：合法 name 必指向**镜像 .rodata**（嵌入 ELF 的 strtab 所在区
/// [_rodata_start, _kernel_edge)）且长度有界。坏 ptr 可落在恒等区内未映射/不可
/// 读页（如 128M 机器 0x88000000 之上）——打印时 OOB 读会触发崩溃现场嵌套
/// fault；长度上限只防破坏值失控。**纯算术判定**（as_ptr/len 已随条目读入
/// 寄存器，不触碰 name 内容字节），panic 现场调用安全。
fn name_in_range(nm: &str) -> bool {
    unsafe extern "C" {
        static _rodata_start: u8;
        static _kernel_edge: u8;
    }
    let (rs, ke) = (
        (&raw const _rodata_start).addr(),
        (&raw const _kernel_edge).addr(),
    );
    let np = nm.as_ptr() as usize;
    let nl = nm.len();
    nl <= 4096 && np >= rs && np.saturating_add(nl) <= ke
}

impl ElfTable {
    /// 从 .symtab+.strtab 构建：只留 STT_FUNC，按 addr 升序；空表 → None。
    /// 表一次建成 Box::leak（'static，永不回收）。
    pub fn from_sections(symtab: &'static [u8], strtab: &'static [u8]) -> Option<ElfTable> {
        let mut entries = Vec::new();
        let n = symtab.len() / SYM_SIZE;
        for i in 0..n {
            let o = i * SYM_SIZE;
            if symtab[o + SYM_INFO] & ST_INFO_TYPE != STT_FUNC {
                continue;
            }
            let value = u64(symtab, o + SYM_VALUE) as usize;
            if value == 0 {
                continue; // 地址 0 的哨兵符号无意义
            }
            let name = name_at(strtab, u32(symtab, o + SYM_NAME) as usize);
            entries.push(Entry {
                addr: VirtAddr::from_raw(value),
                name,
            });
        }
        if entries.is_empty() {
            return None;
        }
        entries.sort_by_key(|e| e.addr.as_usize());
        Some(ElfTable {
            entries: Box::leak(entries.into_boxed_slice()),
        })
    }

    /// 从已按 addr 升序的静态切片构建（调用方保证排序；无排序检查）。
    /// 供内核关键入口表（编译期闭合）使用。
    pub const fn from_entries(entries: &'static [Entry]) -> ElfTable {
        ElfTable { entries }
    }

    /// 二分查最近 ≤ a 的符号；命中 → (名字, 距符号头偏移)。
    ///
    /// 上界约束：a 必须落在符号的「活动区间」内——下一符号起点之前；表尾符号
    /// 用 [`TAIL_SPAN`] 兜底。超出即 None（调用方打印裸 hex）：地址高于表内
    /// 全部符号时不再回退到「表尾符号 + 无意义大偏移」（回溯扫描把栈数据当
    /// 返回地址、以及 `memset+0x8fc5d834` 式标签的病根）。
    pub fn lookup(&self, a: VirtAddr) -> Option<(&'static str, usize)> {
        let target = a.as_usize();
        let mut lo = 0usize;
        let mut hi = self.entries.len();
        let mut found: Option<usize> = None;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.entries[mid].addr.as_usize() <= target {
                found = Some(mid);
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let i = found?;
        let base = self.entries[i].addr.as_usize();
        // 活动区间：下一符号起点；表尾（无下一符号）→ 保守跨度兜底。
        let end = self
            .entries
            .get(i + 1)
            .map(|e| e.addr.as_usize())
            .unwrap_or(base.saturating_add(TAIL_SPAN));
        if target >= end {
            return None;
        }
        // name 防御：条目内存可能被越界写破坏（name 的 ptr/len 坏值会在符号
        // 打印时 OOB 读 → 崩溃现场嵌套 fault → panic 卡死/不停机）。合法 name
        // 必指向**镜像 .rodata**（嵌入 ELF 的 strtab 所在区 [_rodata_start,
        // _kernel_edge)）——超出即拒绝（降级裸 hex）。DRAM 宽范围不够：坏 ptr
        // 可落在恒等区内未映射/不可读页（如 128M 机器 0x88000000 之上）。
        let nm = self.entries[i].name;
        // str 引用（含 len）须完全落入 .rodata（符号名 ≤ 几十字节，长度上限
        // 只防破坏值失控）。坏条目降级为 None（裸 hex）——不打印坏 name，
        // 避免崩溃现场 OOB 读触发嵌套 fault 截断转储。
        if !name_in_range(nm) {
            return None;
        }
        Some((nm, target - base))
    }

    /// 完整性体检（debug-only；panic 现场 drop-in 探针）：核对全表条目
    /// name 防御不变量（[`name_in_range`]）+ addr 排序/非零，坏条目计数并打印
    /// 首例地址——**越界写破坏 entries 时自我报告**（写穿源无处可查时，崩溃
    /// 现场至少自报「表已坏、坏在哪」）。纯读零分配（只经 putln! 直写 SBI
    /// 控制台，无锁安全）；release 编译为空、零开销。返回坏条目数。
    #[cfg(debug_assertions)]
    pub fn check_integrity(&self) -> usize {
        let mut bad = 0usize;
        let mut first = 0usize;
        let mut unsorted = 0usize;
        let mut prev = 0usize;
        for e in self.entries.iter() {
            let a = e.addr.as_usize();
            if a == 0 || !name_in_range(e.name) {
                bad += 1;
                if first == 0 {
                    first = a;
                }
            }
            if a < prev {
                unsorted += 1;
            }
            prev = a;
        }
        if bad > 0 {
            crate::putln!(
                "[table] integrity: {bad}/{} entries name/addr broken (first {first:#x}); {unsorted} unsorted",
                self.entries.len()
            );
        } else if unsorted > 0 {
            crate::putln!("[table] integrity: entries addr unsorted ({unsorted} inversions)");
        }
        bad
    }
}

/// 表尾符号的保守活动跨度（有下一符号时不用）：函数体最大限度，超出即视为
/// 「地址不在任何符号区间内」（栈数据/垃圾字走同一判定）→ [`ElfTable::lookup`]
/// 返回 None。溢出位仅 4B 对齐指令，16 KiB 远超任何单函数尾部的现实跨度。
const TAIL_SPAN: usize = 0x4000;

// ── 地址域判定 ─────────────────────────────────────────────────
//
// 内核/用户域判定下沉为 `VirtAddr::is_kernel()`（addr.rs）：Sv39 高半区
// **或**内核镜像恒等区 [_kernel_start, _kernel_edge) 双段判定。镜像恒等映射
// 落在低半区——纯半区判定（is_user/bit38）会把内核镜像地址（sepc/kbt 帧
// 0x8020xxxx）误判为用户域、分派查错表（见 [`resolve`]）。

// ── 内核表（挂 kernel team，见 work/team::kernel）──────────────

/// 内核符号表单例（懒建一次）：方案 (b) 编译期闭合的小表。
///
/// 不扫 .symtab、不引远端链接符号：直接对关键入口函数 `$f as usize` 取址
/// （链接期解析、恒对应当前镜像，永不过期）。只列调试最关心的入口，够回答
/// 「崩在哪个子系统/哪条路径」；深层 helper 查不到（打印裸 hex）。
///
/// 生命周期**独立于 KERNEL_TEAM**：内容全是链接期常量，本模块 OnceLock 懒建
/// 一次、按值缓存——`unit::init` 自身失败的早期 panic（团队尚未注入）崩溃
/// 路径也能经它符号化，绝不触碰团队单例；`init_kernel` 的团队字段与
/// [`resolve`] 同源取自本表。
static KERNEL_TABLE: OnceLock<ElfTable> = OnceLock::new();
pub fn kernel_table() -> Option<ElfTable> {
    Some(*KERNEL_TABLE.get_or_init(|| {
        let mut v = Vec::new();
        macro_rules! sym {
            ($($n:literal : $f:expr);+ $(;)?) => {$(
                v.push(Entry {
                    addr: VirtAddr::from_raw($f as *const () as usize),
                    name: $n,
                });
            )*};
        }
        sym! {
            "panic_handler" : crate::runtime::diagnose::halt::panic_handler;
            "trap_handler"  : crate::runtime::switcher::trap::trap_handler;
            "boot_main"     : crate::boot::boot_main;
            "restore"       : crate::runtime::switcher::trampoline::restore;
            "sched_run"     : crate::work::room::scheduler::run;
            "sched_idle"    : crate::work::room::scheduler::idle;
            "sched_starve"  : crate::work::room::scheduler::starve;
            "sched_reap"    : crate::work::room::scheduler::reap;
            "sched_park"    : crate::work::room::scheduler::park;
            "sched_unpark"  : crate::work::room::scheduler::unpark;
            "page_fault"    : crate::memory::manager::fault::handle_page_fault;
        }
        v.sort_by_key(|e| e.addr.as_usize());
        ElfTable::from_entries(Box::leak(v.into_boxed_slice()))
    }))
}

// ── 解析器（地址域 → 选表）────────────────────────────────────

/// 地址 → (符号, 偏移)：内核域查内核表（unit::team::kernel），用户域查运行 team 表。
/// 无表/无命中 → None（调用方打印裸 hex）。域判定走 [`VirtAddr::is_kernel`]
/// （高半区 + 镜像恒等区，见 addr.rs）——不用 `is_user` 单半区判定，防镜像
/// 地址查错表。
pub fn resolve(
    addr: VirtAddr,
    team: Option<&crate::work::unit::team::Team>,
) -> Option<(&'static str, usize)> {
    if addr.is_kernel() {
        // 内核表独立挂本模块（编译期闭合、与初始化顺序无关）——早期 panic
        // （unit::init 自身失败）也能符号化；无表 → None，调用方打印裸 hex。
        kernel_table()?.lookup(addr)
    } else {
        team?.elftable.as_ref()?.lookup(addr)
    }
}

/// 地址符号化文本（诊断族公共显示）：命中出「func+0xoff」（demangle），未命中
/// 裸 hex。`tbl` = 任务自己的表（ubt 用，直接 `lookup`）；`None` = 走
/// [`resolve`] 的域判定——内核域恒内核表、用户域退化裸 hex（如 depend 的
/// 内核锁地址、scene 的 kbt/csr 行）。
pub(crate) fn symbol(va: VirtAddr, tbl: Option<&ElfTable>) -> String {
    let hit = match tbl {
        Some(t) => t.lookup(va),
        None => resolve(va, None),
    };
    match hit {
        Some((name, off)) => format!("{}+{off:#x}", rustc_demangle::demangle(name)),
        None => format!("{:#x}", va.as_usize()),
    }
}

//! elftable — 从 ELF 的 .symtab+.strtab 读出的符号表（符号化）。
//!
//! 职责：给定 .symtab + .strtab，产出可按地址二分查询的符号表。名字是 strtab 里
//! 切出的 &'static str（零拷贝）；表一次建成后 Box::leak。

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::memory::manager::addr::VirtAddr;

/// 一条符号（STT_FUNC；表内按 addr 升序）。
pub struct Entry {
    pub addr: VirtAddr,
    pub name: &'static str,
}

/// 符号表 — 有序 Entry 切片（升序，二进制查找见 [`ElfTable::lookup`]）。
pub struct ElfTable {
    entries: &'static [Entry],
}

// ELF64 符号表布局（Elf64_Sym，24 B/条；按字节 + LE 读取）。
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
// 内核/用户域判定走 [`VirtAddr::is_kernel`]（高半区 + 镜像恒等区）。

// ── 内核表 ──

/// 构建内核关键入口表（编译期闭合的小表：直接对函数 `$f as usize` 取址，
/// 不扫 .symtab）。只列调试最关心的入口；深层 helper 查不到（打印裸 hex）。
pub fn kernel_table() -> Option<ElfTable> {
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
    Some(ElfTable::from_entries(Box::leak(v.into_boxed_slice())))
}

// ── 符号化（表显式传入；域路由在消费方，本模块不依赖 team）──────

/// 地址符号化文本：表中命中出「func+0xoff」（demangle），未命中裸 hex。
pub(crate) fn symbol(va: VirtAddr, tbl: Option<&ElfTable>) -> String {
    match tbl.and_then(|t| t.lookup(va)) {
        Some((name, off)) => format!("{}+{off:#x}", rustc_demangle::demangle(name)),
        None => format!("{:#x}", va.as_usize()),
    }
}

/// 域路由查询：内核地址查 kernel_tbl、用户地址查 user_tbl；所选表空 → None。
/// 路由决策（谁提供哪张表）归调用方——解掉 elftable → team 的依赖环；
/// 内核表随内核团队挂载，由消费方取（`team::kernel().elftable`）。
pub fn routed(
    va: VirtAddr,
    kernel_tbl: Option<&ElfTable>,
    user_tbl: Option<&ElfTable>,
) -> Option<(&'static str, usize)> {
    if va.is_kernel() {
        kernel_tbl?.lookup(va)
    } else {
        user_tbl?.lookup(va)
    }
}

/// 域路由符号化文本（[`routed`] + 格式；未命中裸 hex）。
pub fn routed_symbol(
    va: VirtAddr,
    kernel_tbl: Option<&ElfTable>,
    user_tbl: Option<&ElfTable>,
) -> String {
    match routed(va, kernel_tbl, user_tbl) {
        Some((name, off)) => format!("{}+{off:#x}", rustc_demangle::demangle(name)),
        None => format!("{:#x}", va.as_usize()),
    }
}

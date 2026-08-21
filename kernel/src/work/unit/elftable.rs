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
//! 存储挂在 Team.elftable（含 kernel team，见 work/team）；resolve(addr, team) 按
//! 地址域（内核高半区/镜像恒等区 → 内核表，用户区 → 运行 team 表）选表查询。

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::memory::manager::addr::VirtAddr;

/// 一条符号（STT_FUNC；表内按 addr 升序）。
pub struct Entry {
    pub addr: VirtAddr,
    pub name: &'static str,
}

/// 符号表 — 有序 Entry 切片。
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
        Some((
            self.entries[i].name,
            target - self.entries[i].addr.as_usize(),
        ))
    }
}

// ── 地址域判定 ─────────────────────────────────────────────────

/// Sv39 内核高半区起点（与 work::unit::space::KERNEL_BASE 一致）。
const KERNEL_HIGH: usize = 0xFFFF_FFC0_0000_0000;

/// 是否内核域地址：高半区，或内核镜像恒等区 [_kernel_start,_kernel_edge)。
pub fn is_kernel_addr(addr: usize) -> bool {
    if addr >= KERNEL_HIGH {
        return true;
    }
    unsafe extern "C" {
        static _kernel_start: u8;
        static _kernel_edge: u8;
    }
    let s = (&raw const _kernel_start).addr();
    let e = (&raw const _kernel_edge).addr();
    addr >= s && addr < e
}

// ── 内核表（挂 kernel team，见 work/team::kernel）──────────────

/// 内核符号表（挂 kernel team）——方案 (b)：编译期闭合的小表。
///
/// 不扫 .symtab、不引远端链接符号：直接对关键入口函数 `$f as usize` 取址
/// （链接期解析、恒对应当前镜像，永不过期）。只列调试最关心的入口，够回答
/// 「崩在哪个子系统/哪条路径」；深层 helper 查不到（打印裸 hex）。
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

// ── 解析器（地址域 → 选表）────────────────────────────────────

/// 地址 → (符号, 偏移)：内核域查内核表（unit::team::kernel），用户域查运行 team 表。
/// 无表/无命中 → None（调用方打印裸 hex）。
pub fn resolve(
    addr: VirtAddr,
    team: Option<&crate::work::unit::team::Team>,
) -> Option<(&'static str, usize)> {
    if is_kernel_addr(addr.as_usize()) {
        crate::work::unit::team::kernel()
            .elftable
            .as_ref()?
            .lookup(addr)
    } else {
        let t = team?;
        t.elftable.as_ref()?.lookup(addr)
    }
}

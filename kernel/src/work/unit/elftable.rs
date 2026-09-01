//! elftable — 从 ELF 的 .symtab+.strtab 读出的符号表（符号化）。
//!
//! 职责：给定 .symtab + .strtab，产出可按地址二分查询的符号表。名字是 strtab 里
//! 切出的 &'static str（零拷贝，指向镜像 .rodata）；Entry 数组由表自有
//! （Box，随持有者回收——不泄漏）。

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

/// 符号表 — 有序 Entry（升序，二进制查找见 [`ElfTable::lookup`]）。
/// Entry 数组**自有**（随持有者回收，不泄漏）；name 仍 `&'static` 零拷贝指向
/// 镜像 .rodata（见 [`name_in_range`]）。
pub struct ElfTable {
    entries: Box<[Entry]>,
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
    /// Entry 数组由表自有（[`Arc`] 持有者回收），symtab/strtab 切片零拷贝。
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
            entries: entries.into_boxed_slice(),
        })
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
        let base = self.entries[i].addr.as_usize();
        let end = self
            .entries
            .get(i + 1)
            .map(|e| e.addr.as_usize())
            .unwrap_or(base.saturating_add(TAIL_SPAN));
        if target >= end {
            return None;
        }
        let nm = self.entries[i].name;
        if !name_in_range(nm) {
            return None;
        }
        Some((nm, target - base))
    }

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

const TAIL_SPAN: usize = 0x4000;

pub(crate) fn symbol(va: VirtAddr, tbl: Option<&ElfTable>) -> String {
    match tbl.and_then(|t| t.lookup(va)) {
        Some((name, off)) => format!("{:#}+{off:#x}", rustc_demangle::demangle(name)),
        None => format!("{:#x}", va.as_usize()),
    }
}

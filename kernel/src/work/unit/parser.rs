//! ELF 程序解析器 — 纯解析核心，零副作用。
//!
//! 只做「读字节 → 出配方」：验头、列段（PT_LOAD）、验段、取入口；可离线单测。
//!
//! 公开面只留 parse；check/collect/entry 为核心内部原语，不单独暴露，
//! 保证产物 ParsedProgram 只能以「已验段」形态存在（不变量成为类型义务）。
//!
//! fack 0.2.0 语法：元组变体用位置式 {0}/{1}，勿用 {_0}。
use alloc::vec::Vec;
use erra::ResultExt;
use fack::prelude::Error;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

// ── ELF64 布局常量（按字节 + LE 读取，不做结构体整读）──────────

/// ELF 头长（64 B）。
const EHDR: usize = 64;
/// e_ident[0..4] 魔数.
const E_MAGIC: u32 = 0x464C_457F;
/// e_ident[4] class：64 位。
const ELFCLASS64: u8 = 2;
/// e_ident[5] data：小端。
const ELFDATA2LSB: u8 = 1;
/// e_machine：RISC-V。
const EM_RISCV: u16 = 243;
/// e_type：可执行。
const ET_EXEC: u16 = 2;
/// e_type：动态（PIE）。
const ET_DYN: u16 = 3;
/// p_type：可装载段。
const PT_LOAD: u32 = 1;
/// p_flags 位。
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

// Elf64_Phdr 内字段偏移（56 B 条目）。
const PH_TYPE: usize = 0;
const PH_FLAGS: usize = 4;
const PH_OFFSET: usize = 8;
const PH_VADDR: usize = 16;
const PH_FILESZ: usize = 32;
const PH_MEMSZ: usize = 40;
const PH_ENTSIZE: usize = 56;

// Elf64_Shdr 内字段偏移（64 B 条目；仅符号表读取所需）。
const SH_TYPE: usize = 4;
const SH_OFFSET: usize = 24;
const SH_SIZE: usize = 32;
const SH_LINK: usize = 40;
const SHT_SYMTAB: u32 = 2;

// ── 字节读取（带越界由调用方保证）─────────────────────────────

fn u16(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}

fn u32(bytes: &[u8], off: usize) -> u32 {
    let b = |i: usize| bytes[off + i];
    u32::from_le_bytes([b(0), b(1), b(2), b(3)])
}

fn u64(bytes: &[u8], off: usize) -> u64 {
    let b = |i: usize| bytes[off + i];
    u64::from_le_bytes([b(0), b(1), b(2), b(3), b(4), b(5), b(6), b(7)])
}

// ── 失败域 ────────────────────────────────────────────────────

/// 解析失败域 — 失败显式承载，绝不 panic。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    #[error("not an ELF (bad magic)")]
    BadMagic,
    #[error("not a 64-bit ELF class")]
    UnsupportedClass,
    #[error("unsupported byte order")]
    WrongEndian,
    #[error("unsupported machine: {0}")]
    UnsupportedMachine(u16),
    #[error("unsupported ELF type: {0}")]
    UnsupportedType(u16),
    #[error("truncated ELF: need {0}, have {1}")]
    Truncated(usize, usize),
    #[error("segment not page-aligned at {0:#x}")]
    BadAlign(usize),
    #[error("segment has writable+executable permissions")]
    BadPerms,
    #[error("segment memsz < filesz")]
    BssUnderflow,
    #[error("segment range overflows address space")]
    Overflow,
    #[error("no .symtab section")]
    NoSymtab,
}

/// 解析结果 — erra 带调用点上下文（匹配 MapResult/SResult 组合）。
pub type ParseResult<T> = erra::Result<T, ParseError>;

// ── 核心结构 ──────────────────────────────────────────────────

/// 验头产物（核心内部搬运，不进公开契约）。
struct Header {
    entry: usize,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
    pie: bool,
}

/// 待装载段 — 校验后的终态。不变量（构造义务）：memsz >= filesz、flags 无 X⊓W。
pub struct LoadSegment {
    pub vaddr: VirtAddr,
    pub offset: usize,
    pub filesz: usize,
    pub memsz: usize,
    pub flags: PteFlags,
}

/// 解析产物 — 入口 + 全部待装载段。
pub struct ParsedProgram {
    pub entry: VirtAddr,
    #[allow(unused)]
    pub pie: bool,
    pub segments: Vec<LoadSegment>,
}

// ── 核心原语 ──────────────────────────────────────────────────

/// 验头：校验 ELF 属性，产出程序头表信息与入口。
fn check(bytes: &[u8]) -> Result<Header, ParseError> {
    if bytes.len() < EHDR {
        return Err(ParseError::Truncated(EHDR, bytes.len()));
    }
    if u32(bytes, 0) != E_MAGIC {
        return Err(ParseError::BadMagic);
    }
    if bytes[4] != ELFCLASS64 {
        return Err(ParseError::UnsupportedClass);
    }
    if bytes[5] != ELFDATA2LSB {
        return Err(ParseError::WrongEndian);
    }
    let e_type = u16(bytes, 16);
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(ParseError::UnsupportedType(e_type));
    }
    let e_machine = u16(bytes, 18);
    if e_machine != EM_RISCV {
        return Err(ParseError::UnsupportedMachine(e_machine));
    }
    Ok(Header {
        entry: u64(bytes, 24) as usize,
        phoff: u64(bytes, 32) as usize,
        phentsize: u16(bytes, 54) as usize,
        phnum: u16(bytes, 56) as usize,
        pie: e_type == ET_DYN,
    })
}

/// 列段 + 验段：抽出 PT_LOAD，逐一校验到终态 LoadSegment。
fn collect(bytes: &[u8], h: &Header) -> Result<Vec<LoadSegment>, ParseError> {
    if h.phentsize < PH_ENTSIZE {
        return Err(ParseError::Truncated(PH_ENTSIZE, h.phentsize));
    }
    let table_end = h
        .phentsize
        .checked_mul(h.phnum)
        .and_then(|n| h.phoff.checked_add(n))
        .ok_or(ParseError::Overflow)?;
    if table_end > bytes.len() {
        return Err(ParseError::Truncated(table_end, bytes.len()));
    }

    let mut loads = Vec::new();
    for i in 0..h.phnum {
        let base = h.phoff + i * h.phentsize;
        if u32(bytes, base + PH_TYPE) != PT_LOAD {
            continue;
        }
        let flags = u32(bytes, base + PH_FLAGS);
        let offset = u64(bytes, base + PH_OFFSET) as usize;
        let vaddr = u64(bytes, base + PH_VADDR) as usize;
        let filesz = u64(bytes, base + PH_FILESZ) as usize;
        let memsz = u64(bytes, base + PH_MEMSZ) as usize;

        // 验段
        if !vaddr.is_multiple_of(PAGE_SIZE) || !offset.is_multiple_of(PAGE_SIZE) {
            return Err(ParseError::BadAlign(vaddr));
        }
        if memsz < filesz {
            return Err(ParseError::BssUnderflow);
        }
        if (flags & (PF_X | PF_W)) == (PF_X | PF_W) {
            return Err(ParseError::BadPerms);
        }
        if memsz > usize::MAX - vaddr {
            return Err(ParseError::Overflow);
        }

        let mut pt = PteFlags::U;
        if flags & PF_R != 0 {
            pt |= PteFlags::R;
        }
        if flags & PF_W != 0 {
            pt |= PteFlags::W;
        }
        if flags & PF_X != 0 {
            pt |= PteFlags::X;
        }
        loads.push(LoadSegment {
            vaddr: VirtAddr::from_raw(vaddr),
            offset,
            filesz,
            memsz,
            flags: pt,
        });
    }
    Ok(loads)
}

/// 取入口：vaddr（pie 时相对基址，由调用方定址叠加）。
fn entry(h: &Header) -> VirtAddr {
    VirtAddr::from_raw(h.entry)
}

// ── 公开原语 ─────────────────────────────────────────────────

/// 解析：验头 → 列/验段 → 入口，组合出唯一合法的 ParsedProgram。
///
/// 失败显式承载 ParseError，附调用点上下文（erra 约定，匹配 MapResult）。
pub fn parse(bytes: &[u8]) -> ParseResult<ParsedProgram> {
    (|| -> Result<ParsedProgram, ParseError> {
        let h = check(bytes)?;
        let loads = collect(bytes, &h)?;
        Ok(ParsedProgram {
            entry: entry(&h),
            pie: h.pie,
            segments: loads,
        })
    })()
    .annotate("parsing ELF program")
}

/// 抽取 .symtab / 关联 .strtab 两个节切片（用户程序符号化用）。
///
/// 纯读取、零拷贝；不含 .symtab → NoSymtab。切片生命周期绑定入参 bytes，
/// 调用方保证其存活至消费结束。
pub fn tables(bytes: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    (|| -> Result<(&[u8], &[u8]), ParseError> {
        check(bytes)?;
        let shoff = u64(bytes, 40) as usize;
        let shentsize = u16(bytes, 58) as usize;
        let shnum = u16(bytes, 60) as usize;
        if shentsize < SH_LINK + 4 {
            return Err(ParseError::Truncated(SH_LINK + 4, shentsize));
        }
        let table_end = shoff
            .checked_add(shentsize.saturating_mul(shnum))
            .ok_or(ParseError::Overflow)?;
        if table_end > bytes.len() {
            return Err(ParseError::Truncated(table_end, bytes.len()));
        }
        for i in 0..shnum {
            let base = shoff + i * shentsize;
            if u32(bytes, base + SH_TYPE) != SHT_SYMTAB {
                continue;
            }
            let sym_off = u64(bytes, base + SH_OFFSET) as usize;
            let sym_size = u64(bytes, base + SH_SIZE) as usize;
            let str_idx = u32(bytes, base + SH_LINK) as usize;
            let sym_end = sym_off.checked_add(sym_size).ok_or(ParseError::Overflow)?;
            if sym_end > bytes.len() {
                return Err(ParseError::Truncated(sym_end, bytes.len()));
            }
            let sbase = shoff + str_idx * shentsize;
            let str_off = u64(bytes, sbase + SH_OFFSET) as usize;
            let str_size = u64(bytes, sbase + SH_SIZE) as usize;
            let str_end = str_off.checked_add(str_size).ok_or(ParseError::Overflow)?;
            if str_end > bytes.len() {
                return Err(ParseError::Truncated(str_end, bytes.len()));
            }
            return Ok((&bytes[sym_off..sym_end], &bytes[str_off..str_end]));
        }
        Err(ParseError::NoSymtab)
    })()
    .annotate("reading ELF symbol table")
}

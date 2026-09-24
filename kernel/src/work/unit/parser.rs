//! ELF 程序解析器 — 纯解析核心，零副作用。
//!
//! 只做「读字节 → 出配方」：验头、列段（PT_LOAD）、验段、取入口；可离线单测。
//!
//! 输入是**两块事实**：文件头一段前缀（`head`，段表必须落在里头）与文件的真实长度
//! （`file_len`，段实体必须落在里头）。二者分开是"镜像不再整份拷进内核"的直接后果
//! ——读的人只取了文件前 [`HEAD`](super::source::HEAD) 字节，段实体要等 loader 逐段
//! 现取。
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
    #[error("segments overlap in file at {0:#x}")]
    Overlap(usize),
    #[error("segment range overflows address space")]
    Overflow,
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

/// 待装载段 — 校验后的终态。不变量（构造义务）：memsz >= filesz、flags 无 X⊓W、
/// `[offset, offset + filesz)` 落在输入字节内（loader 据此切片）。
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
///
/// 两个界各管一件事：**段表**落在 `head` 内（读的人只取了这么多），**段实体**落在
/// `file_len` 内（loader 逐段现取）。
fn collect(head: &[u8], h: &Header, file_len: usize) -> Result<Vec<LoadSegment>, ParseError> {
    if h.phentsize < PH_ENTSIZE {
        return Err(ParseError::Truncated(PH_ENTSIZE, h.phentsize));
    }
    let table_end = h
        .phentsize
        .checked_mul(h.phnum)
        .and_then(|n| h.phoff.checked_add(n))
        .ok_or(ParseError::Overflow)?;
    if table_end > head.len() {
        return Err(ParseError::Truncated(table_end, head.len()));
    }

    let mut loads: Vec<LoadSegment> = Vec::new();
    for i in 0..h.phnum {
        let base = h.phoff + i * h.phentsize;
        if u32(head, base + PH_TYPE) != PT_LOAD {
            continue;
        }
        let flags = u32(head, base + PH_FLAGS);
        let offset = u64(head, base + PH_OFFSET) as usize;
        let vaddr = u64(head, base + PH_VADDR) as usize;
        let filesz = u64(head, base + PH_FILESZ) as usize;
        let memsz = u64(head, base + PH_MEMSZ) as usize;

        // 验段
        if !vaddr.is_multiple_of(PAGE_SIZE) || !offset.is_multiple_of(PAGE_SIZE) {
            return Err(ParseError::BadAlign(vaddr));
        }
        // 文件实体必须整段落在**文件**里（不是落在头窗口里）：loader 按
        // `[offset, offset + filesz)` 向源现取，越界即源答 `false`（用户可控的 PT_LOAD
        // 能构造出来）。need/have 与上面两处截断同形。
        let file_end = offset.checked_add(filesz).ok_or(ParseError::Overflow)?;
        if file_end > file_len {
            return Err(ParseError::Truncated(file_end, file_len));
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
        // 段实体在文件内**两两不重叠**：`Σ filesz ≤ file_len` 这条界靠它成立，而那个和
        // 就是"这份镜像最多让内核从调用方那里读多少字节"——没有它，一份段表可以点一千段
        // 全指同一处，把"调用方的映射即上界"那条封顶击穿。半开区间形，故 `filesz == 0`
        // 的段（`.bss`，常见于紧贴前一段的文件偏移处）天然不参与。
        if filesz > 0 {
            for prev in &loads {
                if prev.filesz > 0 && offset < prev.offset + prev.filesz && prev.offset < file_end {
                    return Err(ParseError::Overlap(offset.max(prev.offset)));
                }
            }
        }

        // 只产 ELF 语义（R/W/X）；U 位是映射策略，由 loader 按空间模式加。
        let mut pt = PteFlags::empty();
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
/// `head` = 文件从偏移 0 起的一段**前缀**（读的人按
/// [`HEAD`](super::source::HEAD) 取的那么多）；`file_len` = 文件的**真实长度**。
/// 旧形状只有 `bytes.len()` 一个界——"只给前缀"之后它会把每一份合法镜像都判成截断。
///
/// 前置：`head` 是文件前缀，且 `head.len() <= file_len`。
///
/// 失败显式承载 ParseError，附调用点上下文（erra 约定，匹配 MapResult）。
pub fn parse(head: &[u8], file_len: usize) -> ParseResult<ParsedProgram> {
    (|| -> Result<ParsedProgram, ParseError> {
        let h = check(head)?;
        let loads = collect(head, &h, file_len)?;
        Ok(ParsedProgram {
            entry: entry(&h),
            pie: h.pie,
            segments: loads,
        })
    })()
    .annotate("parsing ELF program")
}

use alloc::vec::Vec;
use erra::ResultExt;
use fack::prelude::Error;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;

const EHDR: usize = 64;
const E_MAGIC: u32 = 0x464C_457F;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EM_RISCV: u16 = 243;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

const PH_TYPE: usize = 0;
const PH_FLAGS: usize = 4;
const PH_OFFSET: usize = 8;
const PH_VADDR: usize = 16;
const PH_FILESZ: usize = 32;
const PH_MEMSZ: usize = 40;
const PH_ENTSIZE: usize = 56;

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

pub type ParseResult<T> = erra::Result<T, ParseError>;

struct Header {
    entry: usize,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
    pie: bool,
}

pub struct LoadSegment {
    pub vaddr: VirtAddr,
    pub offset: usize,
    pub filesz: usize,
    pub memsz: usize,
    pub flags: PteFlags,
}

pub struct ParsedProgram {
    pub entry: VirtAddr,
    #[allow(unused)]
    pub pie: bool,
    pub segments: Vec<LoadSegment>,
}

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

        if !vaddr.is_multiple_of(PAGE_SIZE) || !offset.is_multiple_of(PAGE_SIZE) {
            return Err(ParseError::BadAlign(vaddr));
        }
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
        if filesz > 0 {
            for prev in &loads {
                if prev.filesz > 0 && offset < prev.offset + prev.filesz && prev.offset < file_end {
                    return Err(ParseError::Overlap(offset.max(prev.offset)));
                }
            }
        }

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

fn entry(h: &Header) -> VirtAddr {
    VirtAddr::from_raw(h.entry)
}

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

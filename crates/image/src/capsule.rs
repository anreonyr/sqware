use elf::{ElfBytes, endian::LittleEndian, file::Class};
use env::ledger::capsule::{HEADER, MAGIC, PAGE, RECORD};

#[derive(Debug)]
pub(crate) enum Error {
    Format,
    Unsupported,
    Range,
    Permissions,
    Overlap,
    Entry,
    Memory,
}

#[derive(Debug)]
struct Region {
    va: usize,
    size: usize,
    data_size: usize,
    flags: u64,
    file_offset: usize,
    file_size: usize,
    prefix: usize,
}

struct Plan {
    entry: usize,
    regions: Vec<Region>,
}

fn parse(bytes: &[u8]) -> Result<Plan, Error> {
    let file = ElfBytes::<LittleEndian>::minimal_parse(bytes).map_err(|_| Error::Format)?;
    let header = &file.ehdr;
    if header.class != Class::ELF64
        || header.e_type != elf::abi::ET_EXEC
        || header.e_machine != elf::abi::EM_RISCV
        || header.version != 1
        || header.e_ehsize != 64
        || header.e_phentsize != 56
        || header.e_phnum == 0
    {
        return Err(Error::Unsupported);
    }
    let entry = usize::try_from(header.e_entry).map_err(|_| Error::Range)?;
    let segments = file.segments().ok_or(Error::Format)?;
    let count = usize::from(header.e_phnum);
    let mut regions: Vec<Region> = Vec::new();
    regions.try_reserve(count).map_err(|_| Error::Memory)?;
    let mut valid_entry = false;
    for i in 0..count {
        let segment = segments.get(i).map_err(|_| Error::Format)?;
        let kind = segment.p_type;
        if kind == elf::abi::PT_DYNAMIC || kind == elf::abi::PT_INTERP {
            return Err(Error::Unsupported);
        }
        if kind != elf::abi::PT_LOAD {
            continue;
        }
        let permissions = usize::try_from(segment.p_flags).map_err(|_| Error::Range)?;
        let offset = usize::try_from(segment.p_offset).map_err(|_| Error::Range)?;
        let address = usize::try_from(segment.p_vaddr).map_err(|_| Error::Range)?;
        let file_size = usize::try_from(segment.p_filesz).map_err(|_| Error::Range)?;
        let memory_size = usize::try_from(segment.p_memsz).map_err(|_| Error::Range)?;
        let alignment = usize::try_from(segment.p_align).map_err(|_| Error::Range)?;
        if memory_size < file_size
            || offset
                .checked_add(file_size)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(Error::Range);
        }
        if alignment > 1
            && (!alignment.is_power_of_two() || address % alignment != offset % alignment)
        {
            return Err(Error::Format);
        }
        if permissions & !7 != 0 || permissions & 4 == 0 || permissions & 3 == 3 {
            return Err(Error::Permissions);
        }
        if memory_size == 0 {
            continue;
        }
        let end = address.checked_add(memory_size).ok_or(Error::Range)?;
        let flags =
            ((permissions & 4) >> 1 | (permissions & 2) << 1 | (permissions & 1) << 3) as u64;
        if flags & 8 != 0
            && entry >= address
            && entry
                .checked_add(2)
                .is_some_and(|end| end <= address + file_size)
        {
            valid_entry = true;
        }
        let va = address / PAGE * PAGE;
        let end = end.checked_next_multiple_of(PAGE).ok_or(Error::Range)?;
        if va == 0
            || regions
                .iter()
                .any(|region| va < region.va + region.size && region.va < end)
        {
            return Err(Error::Overlap);
        }
        let prefix = address - va;
        let size = end - va;
        let data_size = if flags & 8 != 0 {
            size
        } else if file_size == 0 {
            0
        } else {
            prefix
                .checked_add(file_size)
                .and_then(|n| n.checked_next_multiple_of(PAGE))
                .ok_or(Error::Range)?
        };
        regions.push(Region {
            va,
            size,
            data_size,
            flags,
            file_offset: offset,
            file_size,
            prefix,
        });
    }
    if !valid_entry || !entry.is_multiple_of(2) {
        return Err(Error::Entry);
    }
    Ok(Plan { entry, regions })
}

pub(crate) fn encode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let plan = parse(bytes)?;
    let table = HEADER
        .checked_add(plan.regions.len().checked_mul(RECORD).ok_or(Error::Range)?)
        .ok_or(Error::Range)?;
    let mut length = table.checked_next_multiple_of(PAGE).ok_or(Error::Range)?;
    for region in &plan.regions {
        length = length.checked_add(region.data_size).ok_or(Error::Range)?;
    }
    let mut out = Vec::new();
    out.try_reserve_exact(length).map_err(|_| Error::Memory)?;
    out.resize(length, 0);
    out[..8].copy_from_slice(&MAGIC);
    out[8..10].copy_from_slice(&1u16.to_le_bytes());
    out[10] = 12;
    out[12..16].copy_from_slice(
        &u32::try_from(plan.regions.len())
            .map_err(|_| Error::Range)?
            .to_le_bytes(),
    );
    out[16..24].copy_from_slice(&(plan.entry as u64).to_le_bytes());
    out[24..32].copy_from_slice(&(length as u64).to_le_bytes());
    let mut payload = table.next_multiple_of(PAGE);
    for (i, region) in plan.regions.iter().enumerate() {
        let at = HEADER + i * RECORD;
        let offset = if region.data_size == 0 { 0 } else { payload };
        for (field, value) in [
            region.va,
            region.size / PAGE,
            region.data_size / PAGE,
            offset,
        ]
        .into_iter()
        .enumerate()
        {
            out[at + field * 8..at + field * 8 + 8].copy_from_slice(&(value as u64).to_le_bytes());
        }
        let permissions = (region.flags >> 1) as u32;
        out[at + 32..at + 36].copy_from_slice(&permissions.to_le_bytes());
        if region.file_size > 0 {
            out[payload + region.prefix..payload + region.prefix + region.file_size]
                .copy_from_slice(&bytes[region.file_offset..region.file_offset + region.file_size]);
        }
        payload += region.data_size;
    }
    Ok(out)
}

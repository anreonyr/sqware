#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use env::ledger::capsule::{HEADER, MAGIC, PAGE, RECORD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Format,
    Unsupported,
    Range,
    Permissions,
    Overlap,
    Entry,
    Memory,
}

#[derive(Debug)]
pub struct Region {
    pub va: usize,
    pub size: usize,
    pub data_size: usize,
    pub flags: u64,
    pub file_offset: usize,
    pub file_size: usize,
    pub prefix: usize,
}

pub struct Plan {
    pub entry: usize,
    pub regions: Vec<Region>,
}

fn number(bytes: &[u8], offset: usize, width: usize) -> Result<usize, Error> {
    let mut raw = [0u8; 8];
    raw[..width].copy_from_slice(
        bytes
            .get(offset..offset.checked_add(width).ok_or(Error::Range)?)
            .ok_or(Error::Format)?,
    );
    usize::try_from(u64::from_le_bytes(raw)).map_err(|_| Error::Range)
}

pub fn parse(bytes: &[u8]) -> Result<Plan, Error> {
    if bytes.len() < 64 || bytes[..6] != [0x7f, b'E', b'L', b'F', 2, 1] {
        return Err(Error::Format);
    }
    if bytes[6] != 1
        || number(bytes, 16, 2)? != 2
        || number(bytes, 18, 2)? != 243
        || number(bytes, 20, 4)? != 1
        || number(bytes, 52, 2)? != 64
    {
        return Err(Error::Unsupported);
    }
    let entry = number(bytes, 24, 8)?;
    let phoff = number(bytes, 32, 8)?;
    let stride = number(bytes, 54, 2)?;
    let count = number(bytes, 56, 2)?;
    if stride != 56
        || count == 0
        || phoff
            .checked_add(count.checked_mul(stride).ok_or(Error::Range)?)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(Error::Format);
    }
    let mut regions: Vec<Region> = Vec::new();
    regions.try_reserve(count).map_err(|_| Error::Memory)?;
    let mut valid_entry = false;
    for i in 0..count {
        let at = phoff + i * stride;
        let kind = number(bytes, at, 4)?;
        if kind == 2 || kind == 3 {
            return Err(Error::Unsupported);
        }
        if kind != 1 {
            continue;
        }
        let permissions = number(bytes, at + 4, 4)?;
        let offset = number(bytes, at + 8, 8)?;
        let address = number(bytes, at + 16, 8)?;
        let file_size = number(bytes, at + 32, 8)?;
        let memory_size = number(bytes, at + 40, 8)?;
        let alignment = number(bytes, at + 48, 8)?;
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

pub fn capsule(bytes: &[u8]) -> Result<Vec<u8>, Error> {
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

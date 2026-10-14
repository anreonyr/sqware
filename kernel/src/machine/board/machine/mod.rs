use crate::core::memory::PAGE_SIZE;
use crate::machine::hart::HartId;
use crate::machine::image::root_stack_edge;

use super::{MachineError, Region};

mod hart_info;
use hart_info::HartInfo;

pub const MAX_RESERVED: usize = 2;

const RESERVED_INITRD: usize = 0;
const RESERVED_DTB: usize = 1;

#[derive(Clone, Copy, Debug)]
pub struct Machine {
    pub hart: HartInfo,
    pub dram: Region,
    pub free: Region,
    pub reserved: [Option<Region>; MAX_RESERVED],
}

impl Machine {
    pub(super) fn parse(dtp: usize, boot: HartId) -> Result<Self, MachineError> {
        let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.map_err(MachineError::DeviceTree)?;
        let hart = HartInfo::parse(&fdt, boot)?;
        let mem = fdt
            .find_node("/memory")
            .and_then(|node| node.reg())
            .and_then(|mut regions| regions.next())
            .ok_or(MachineError::MissingMemory)?;
        let dram_base = mem.starting_address.addr();
        let dram_size = mem
            .size
            .filter(|size| *size > 0)
            .ok_or(MachineError::MissingMemory)?;
        let free_base = root_stack_edge();
        let free_end = dram_base
            .checked_add(dram_size)
            .ok_or(MachineError::InvalidMemory)?;
        unsafe extern "C" {
            static _kernel_base: u8;
        }
        if dram_base > ::core::ptr::addr_of!(_kernel_base) as usize || free_base > free_end {
            return Err(MachineError::InvalidMemory);
        }
        let free_size = free_end - free_base;
        let dtb_size = fdt
            .total_size()
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(MachineError::DtbRegion)?;
        if dtp < dram_base || dtp.checked_add(dtb_size).is_none_or(|end| end > free_end) {
            return Err(MachineError::DtbRegion);
        }
        let initrd = initrd_region(&fdt)?;
        if initrd.is_some_and(|region| region.base < free_base || region.range().end > free_end) {
            return Err(MachineError::Initrd);
        }
        let mut reserved = [None; MAX_RESERVED];
        reserved[RESERVED_INITRD] = initrd;
        reserved[RESERVED_DTB] = Some(Region::new(dtp, dtb_size));
        Ok(Self {
            hart,
            dram: Region::new(dram_base, dram_size),
            free: Region::new(free_base, free_size),
            reserved,
        })
    }

    pub fn initrd(&self) -> Option<Region> {
        self.reserved[RESERVED_INITRD]
    }

    pub fn dtb(&self) -> Region {
        self.reserved[RESERVED_DTB].expect("machine::init always reserves the DTB")
    }
}

fn initrd_region(fdt: &fdt::Fdt) -> Result<Option<Region>, MachineError> {
    let Some(chosen) = fdt.find_node("/chosen") else {
        return Ok(None);
    };
    let start = chosen.property("linux,initrd-start");
    let end = chosen.property("linux,initrd-end");
    if start.is_none() && end.is_none() {
        return Ok(None);
    }
    let start = start
        .and_then(|p| p.as_usize())
        .ok_or(MachineError::Initrd)?;
    let end = end.and_then(|p| p.as_usize()).ok_or(MachineError::Initrd)?;
    if start == 0 || end <= start || !start.is_multiple_of(PAGE_SIZE) {
        return Err(MachineError::Initrd);
    }
    let end = end
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or(MachineError::Initrd)?;
    Ok(Some(Region::new(start, end - start)))
}

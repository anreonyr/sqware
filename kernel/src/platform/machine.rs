use core::ops;

use fack::prelude::Error;

use crate::layout::{MAX_HART_SLOTS, root_stack_edge};
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: usize,
    pub size: usize,
}

impl Region {
    pub fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }
    pub fn range(&self) -> ops::Range<usize> {
        self.base..self.base + self.size
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HartInfo {
    pub count: usize,
    pub hertz: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Machine {
    pub hart: HartInfo,
    pub dram: Region,
    pub free: Region,
    pub reserved: [Option<Region>; MAX_RESERVED],
}

pub const MAX_RESERVED: usize = 2;

const RESERVED_INITRD: usize = 0;
const RESERVED_DTB: usize = 1;

impl Machine {
    pub fn initrd(&self) -> Option<Region> {
        self.reserved[RESERVED_INITRD]
    }

    pub fn dtb(&self) -> Region {
        self.reserved[RESERVED_DTB].expect("machine::init always reserves the DTB")
    }
}

static MACHINE: OnceLock<Machine> = OnceLock::new();

#[derive(Error, Debug)]
pub enum MachineError {
    #[error("invalid device tree: {0}")]
    DeviceTree(fdt::FdtError),
    #[error("device tree has no /cpus node")]
    MissingCpus,
    #[error("device tree has no usable memory region")]
    MissingMemory,
    #[error("RAM does not contain the kernel and root stack")]
    InvalidMemory,
    #[error("invalid hart topology: count={count}, boot={boot}")]
    Harts { count: usize, boot: usize },
    #[error("invalid initrd region")]
    Initrd,
    #[error("device tree region is outside RAM")]
    DtbRegion,
}

pub fn init(dtp: usize) -> Result<(), MachineError> {
    // SAFETY: firmware supplies a readable DTB pointer.
    let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.map_err(MachineError::DeviceTree)?;
    let cpus = fdt.find_node("/cpus").ok_or(MachineError::MissingCpus)?;
    let count = cpus
        .children()
        .filter(|node| node.name.split('@').next() == Some("cpu"))
        .count();
    let boot = crate::hart::hart_id().get();
    if count == 0 || count > MAX_HART_SLOTS || boot >= count {
        return Err(MachineError::Harts { count, boot });
    }
    let mut seen = [false; MAX_HART_SLOTS];
    for node in cpus
        .children()
        .filter(|node| node.name.split('@').next() == Some("cpu"))
    {
        let id = node
            .property("reg")
            .and_then(|property| property.as_usize());
        let Some(id) = id.filter(|id| *id < count) else {
            return Err(MachineError::Harts { count, boot });
        };
        if core::mem::replace(&mut seen[id], true) {
            return Err(MachineError::Harts { count, boot });
        }
    }
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
    if dram_base > core::ptr::addr_of!(_kernel_base) as usize || free_base > free_end {
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
    assert!(
        MACHINE
            .set(Machine {
                dram: Region::new(dram_base, dram_size),
                free: Region::new(free_base, free_size),
                hart: HartInfo {
                    count,
                    hertz: hertz(&fdt)
                },
                reserved,
            })
            .is_ok(),
        "machine already initialized"
    );
    Ok(())
}

pub fn info() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
}

pub(crate) fn dram_edge() -> Option<usize> {
    MACHINE.get().map(|m| m.dram.range().end)
}

fn hertz(fdt: &fdt::Fdt) -> usize {
    fdt.find_node("/cpus")
        .and_then(|n| n.property("timebase-frequency"))
        .map(|p| match p.value.len() {
            4 => u32::from_be_bytes([p.value[0], p.value[1], p.value[2], p.value[3]]) as usize,
            8 => {
                let b = p.value;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
            }
            _ => 0,
        })
        .unwrap_or(0)
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

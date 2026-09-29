use core::ops;

use crate::layout::root_stack_edge;
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

pub fn init(dtp: usize) {
    let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.expect("invalid device tree blob");

    let count = fdt.cpus().count();

    let mem = fdt
        .memory()
        .regions()
        .next()
        .expect("device tree has no /memory node");
    let dram_base = mem.starting_address.addr();
    let dram_size = mem.size.unwrap_or(0);
    let hertz = hertz(&fdt);
    let hart = HartInfo { count, hertz };

    let free_base = root_stack_edge();
    let free_end = dram_base + dram_size;
    let free_size = free_end - free_base;

    let initrd = initrd_region(&fdt);
    let dtb = Region::new(dtp, dtb_size(dtp as *const u8));

    let mut reserved = [None; MAX_RESERVED];
    reserved[RESERVED_INITRD] = initrd;
    reserved[RESERVED_DTB] = Some(dtb);

    MACHINE
        .set(Machine {
            dram: Region::new(dram_base, dram_size),
            free: Region::new(free_base, free_size),
            hart,
            reserved,
        })
        .unwrap()
}

fn dtb_size(dtp: *const u8) -> usize {
    // SAFETY: dtp 是 boot 交上来的 DTB 首址，前 8 字节是 magic + totalsize
    let total = unsafe { core::ptr::read_unaligned(dtp.add(4).cast::<u32>()) };
    (u32::from_be(total) as usize).next_multiple_of(PAGE_SIZE)
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

fn initrd_region(fdt: &fdt::Fdt) -> Option<Region> {
    let chosen = fdt.find_node("/chosen")?;
    let start = chosen.property("linux,initrd-start")?.as_usize()?;
    let end = chosen.property("linux,initrd-end")?.as_usize()?;
    if start == 0 || end <= start || !start.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let end = end.next_multiple_of(PAGE_SIZE);
    Some(Region::new(start, end - start))
}

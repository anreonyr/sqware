use riscv::register::satp;

use crate::layout;
use crate::lock::OnceLock;
use crate::memory::PAGE_SHIFT;

use super::addr::VirtAddr;
use super::entry::PteFlags;
use super::table::TableNode;

#[derive(fack::prelude::Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatpError {
    #[error("no supported page table mode")]
    Unsupported,
    #[error("page table mode probe ran out of memory")]
    OutOfMemory,
}

static MODE: OnceLock<satp::Mode> = OnceLock::new();

pub fn detect() -> Result<satp::Mode, SatpError> {
    for candidate in [satp::Mode::Sv57, satp::Mode::Sv48, satp::Mode::Sv39] {
        match try_mode(candidate) {
            Ok(()) => {
                MODE.set(candidate).expect("mode: detect is single-shot");
                return Ok(candidate);
            }
            Err(SatpError::Unsupported) => {},
            Err(error) => return Err(error),
        }
    }
    Err(SatpError::Unsupported)
}

pub fn mode() -> satp::Mode {
    *MODE.get().unwrap_or(&satp::Mode::Sv39)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geo {
    pub levels: u8,
    pub va_bits: u8,
}

impl Geo {
    #[inline]
    pub fn split_bit(self) -> u8 {
        self.va_bits - 1
    }
}

pub fn geometry(mode: satp::Mode) -> Geo {
    match mode {
        satp::Mode::Sv39 => Geo {
            levels: 3,
            va_bits: 39,
        },
        satp::Mode::Sv48 => Geo {
            levels: 4,
            va_bits: 48,
        },
        satp::Mode::Sv57 => Geo {
            levels: 5,
            va_bits: 57,
        },
        _ => panic!("geometry: unsupported satp mode {mode:?}"),
    }
}

pub fn levels() -> usize {
    geometry(mode()).levels as usize
}

pub fn lower() -> VirtAddr {
    VirtAddr::from_raw(1usize << geometry(mode()).split_bit())
}

pub fn upper() -> VirtAddr {
    VirtAddr::wrap(1usize << geometry(mode()).split_bit())
}

fn try_mode(candidate: satp::Mode) -> Result<(), SatpError> {
    let geo = geometry(candidate);
    let mut root = TableNode::root().map_err(|_| SatpError::OutOfMemory)?;
    unsafe extern "C" {
        static _kernel_base: u8;
    }
    let range = ((&raw const _kernel_base).addr(), layout::root_stack_edge());
    let mut va = range.0 & !(crate::memory::PAGE_SIZE - 1);
    while va < range.1 {
        let ppn = (va >> PAGE_SHIFT) as u64;
        let leaf = root
            .walk_mut(VirtAddr::wrap(va), true, geo.levels as usize)
            .map_err(|_| SatpError::OutOfMemory)?;
        leaf.set(ppn, PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::X);
        va += crate::memory::PAGE_SIZE;
    }
    let ppn = root.ppn();
    // SAFETY: 恒等临时根已覆盖探测执行区；回读后立即写回 Bare
    unsafe {
        satp::set(candidate, 0, ppn);
        core::arch::asm!("sfence.vma");
    }
    let got = satp::read().mode();
    // SAFETY: 立即关闭翻译
    unsafe {
        satp::set(satp::Mode::Bare, 0, 0);
        core::arch::asm!("sfence.vma");
    }
    drop(root);
    if got == candidate {
        Ok(())
    } else {
        Err(SatpError::Unsupported)
    }
}

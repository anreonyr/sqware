use riscv::register::satp;

use crate::core::memory::PAGE_SHIFT;
use crate::machine::image;
use crate::machine::sync::OnceLock;

use super::addr::VirtAddr;
use super::entry::PteFlags;
use super::table::TableNode;

mod error;
mod geometry;

pub use error::SatpError;
pub use geometry::{Geo, geometry};

static MODE: OnceLock<satp::Mode> = OnceLock::new();

pub fn detect() -> Result<satp::Mode, SatpError> {
    for candidate in [satp::Mode::Sv57, satp::Mode::Sv48, satp::Mode::Sv39] {
        match try_mode(candidate) {
            Ok(()) => {
                MODE.set(candidate).expect("mode: detect is single-shot");
                return Ok(candidate);
            }
            Err(SatpError::Unsupported) => {}
            Err(error) => return Err(error),
        }
    }
    Err(SatpError::Unsupported)
}

pub fn mode() -> satp::Mode {
    *MODE.get().unwrap_or(&satp::Mode::Sv39)
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
    let range = ((&raw const _kernel_base).addr(), image::root_stack_edge());
    let mut va = range.0 & !(crate::core::memory::PAGE_SIZE - 1);
    while va < range.1 {
        let ppn = (va >> PAGE_SHIFT) as u64;
        let leaf = root
            .walk_mut(VirtAddr::wrap(va), true, geo.levels as usize)
            .map_err(|_| SatpError::OutOfMemory)?;
        leaf.set(ppn, PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::X);
        va += crate::core::memory::PAGE_SIZE;
    }
    let ppn = root.ppn();
    unsafe {
        satp::set(candidate, 0, ppn);
        core::arch::asm!("sfence.vma");
    }
    let got = satp::read().mode();
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

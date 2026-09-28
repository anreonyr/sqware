use bitflags::bitflags;
use core::fmt;

bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct PteFlags: u64 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Default)]
pub struct PageTableEntry {
    bits: u64,
}

impl PageTableEntry {
    const FLAGS_MASK: u64 = 0x3FF;
    const PPN_SHIFT: usize = 10;

    #[inline(always)]
    pub fn is_valid(self) -> bool {
        self.flags().contains(PteFlags::V)
    }

    #[inline(always)]
    pub fn is_leaf(self) -> bool {
        self.flags()
            .intersects(PteFlags::R | PteFlags::W | PteFlags::X)
    }

    #[inline(always)]
    pub fn is_branch(self) -> bool {
        self.is_valid() && !self.is_leaf()
    }

    #[inline(always)]
    pub fn ppn(self) -> u64 {
        self.bits >> Self::PPN_SHIFT
    }

    #[inline(always)]
    pub fn paddr(self) -> u64 {
        self.ppn() << 12
    }

    #[inline(always)]
    pub fn flags(self) -> PteFlags {
        PteFlags::from_bits_truncate(self.bits & Self::FLAGS_MASK)
    }

    #[inline(always)]
    pub fn set(&mut self, ppn: u64, flags: PteFlags) {
        self.bits = (ppn << Self::PPN_SHIFT) | flags.bits();
    }

    #[inline(always)]
    pub fn set_flags(&mut self, flags: PteFlags) {
        self.bits = (self.bits & !Self::FLAGS_MASK) | flags.bits();
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.bits = 0;
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_valid() {
            write!(f, "PTE(invalid)")
        } else if self.is_branch() {
            write!(f, "PTE(branch → {:#x})", self.paddr())
        } else {
            write!(
                f,
                "PTE({:#x}, ppn={:#x}, flags={:?})",
                self.bits,
                self.ppn(),
                self.flags()
            )
        }
    }
}
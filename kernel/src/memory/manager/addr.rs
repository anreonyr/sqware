use core::ops::{Add, AddAssign, Sub, SubAssign};

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(usize);

impl VirtAddr {
    pub fn from_raw(addr: usize) -> Self {
        let bit = super::mode::geometry(super::mode::mode()).split_bit() as usize;
        let sign = ((addr as isize) << (63 - bit)) >> (63 - bit);
        Self(sign as usize)
    }

    #[inline]
    pub const fn wrap(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub fn vpn(self, level: u8) -> usize {
        (self.0 >> (PAGE_SHIFT + level as usize * 9)) & 0x1FF
    }

    #[inline]
    pub fn offset(self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    #[inline]
    pub fn is_user(self) -> bool {
        let bit = super::mode::geometry(super::mode::mode()).split_bit() as usize;
        (self.0 >> bit) & 1 == 0
    }

    #[inline]
    pub fn is_kernel(self) -> bool {
        if !self.is_user() {
            return true;
        }
        unsafe extern "C" {
            static _kernel_base: u8;
            static _kernel_edge: u8;
        }
        let a = self.0;
        let (s, e) = (
            (&raw const _kernel_base).addr(),
            (&raw const _kernel_edge).addr(),
        );
        a >= s && a < e
    }

    #[inline]
    pub fn page_align(self) -> Self {
        Self(self.0 & !(PAGE_SIZE - 1))
    }

    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl Add<usize> for VirtAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for VirtAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self {
        Self(self.0 - rhs)
    }
}

impl AddAssign<usize> for VirtAddr {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for VirtAddr {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

impl core::fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VA({:#x})", self.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(usize);

impl PhysAddr {
    #[inline]
    pub const fn from_raw(addr: usize) -> Self {
        Self(addr)
    }

    #[inline]
    pub fn is_aligned(self) -> bool {
        self.0 & (PAGE_SIZE - 1) == 0
    }

    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl Add<usize> for PhysAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: usize) -> Self {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for PhysAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: usize) -> Self {
        Self(self.0 - rhs)
    }
}

impl AddAssign<usize> for PhysAddr {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for PhysAddr {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

impl core::fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PA({:#x})", self.0)
    }
}

impl core::fmt::LowerHex for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}
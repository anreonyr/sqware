// 类型安全的虚拟地址 / 物理地址 / 物理页号

use core::ops::{Add, AddAssign, Sub, SubAssign};

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

/// Sv39 虚拟地址。
///
/// 保证规范形式：bits 63:39 全等于 bit 38。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(usize);

impl VirtAddr {
    /// 从原始 usize 构造虚拟地址，按**当前模式**进位到规范形式。
    ///
    /// 进位位 = 分裂位（va_bits−1，见 `mode::Geo`）；对已规范输入恒等。模式
    /// 未探测时兜底 Sv39（进位位 38 = 现状语义，低地址在任意模式进位下不变）。
    pub fn from_raw(addr: usize) -> Self {
        let bit = super::mode::geometry(super::mode::mode()).split_bit() as usize;
        let sign = ((addr as isize) << (63 - bit)) >> (63 - bit);
        Self(sign as usize)
    }

    /// 纯位包装（零进位）— 布局常量构造器。
    ///
    /// # 前置
    ///
    /// 输入必须是全部受支持模式下均已规范的值（顶锚定 bit63=1，或低地址）；
    /// 由布局 const 断言与运行期 `validate()` 验证。
    #[inline]
    pub const fn wrap(addr: usize) -> Self {
        Self(addr)
    }

    /// 提取指定级别的 VPN（9 位索引）。
    ///
    /// `level` 2 → bits 38:30, `level` 1 → bits 29:21, `level` 0 → bits 20:12
    #[inline]
    pub fn vpn(self, level: u8) -> usize {
        (self.0 >> (PAGE_SHIFT + level as usize * 9)) & 0x1FF
    }

    /// 页内偏移（bits 11:0）
    #[inline]
    pub fn offset(self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }

    /// 向下对齐到页边界
    #[inline]
    pub fn page_align(self) -> Self {
        Self(self.0 & !(PAGE_SIZE - 1))
    }

    /// 是否为用户地址（分裂位 = 0，即地址 < 2^split_bit；分割随当前模式）。
    #[inline]
    pub fn is_user(self) -> bool {
        let bit = super::mode::geometry(super::mode::mode()).split_bit() as usize;
        (self.0 >> bit) & 1 == 0
    }

    /// 是否为内核域地址：分裂位以上（当前模式内核半区），**或**内核镜像恒等区
    /// [_kernel_start, _kernel_edge)。
    ///
    /// 两段都要：镜像恒等映射落在**低半区**（0x80200000 起），纯半区判定会误判。
    #[inline]
    pub fn is_kernel(self) -> bool {
        if !self.is_user() {
            return true; // 高半区
        }
        unsafe extern "C" {
            static _kernel_start: u8;
            static _kernel_edge: u8;
        }
        let a = self.0;
        let (s, e) = (
            (&raw const _kernel_start).addr(),
            (&raw const _kernel_edge).addr(),
        );
        a >= s && a < e
    }

    /// 获取原始 usize 值
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

/// 物理地址。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(usize);

impl PhysAddr {
    /// 从原始 usize 构造物理地址。
    #[inline]
    pub const fn from_raw(addr: usize) -> Self {
        Self(addr)
    }

    /// 是否 4 KiB 对齐
    #[inline]
    pub fn is_aligned(self) -> bool {
        self.0 & (PAGE_SIZE - 1) == 0
    }

    /// 向下对齐到页边界
    #[inline]
    #[allow(dead_code)] // 对齐工具预留
    pub fn page_align(self) -> Self {
        Self(self.0 & !(PAGE_SIZE - 1))
    }

    /// 获取原始 usize 值
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

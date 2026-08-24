// 类型安全的虚拟地址 / 物理地址 / 物理页号
//
// Sv39 虚拟地址规范：
//   bits 63:39 — 必须全等于 bit 38（符号扩展）
//   bits 38:30 — VPN[2]  (L2 索引)
//   bits 29:21 — VPN[1]  (L1 索引)
//   bits 20:12 — VPN[0]  (L0 索引)
//   bits 11:0  — 页内偏移
//
// VPN[2] 决定地址空间：
//   0x000 (0-255)   — 用户半区  (0x0000_0000_0000_0000 .. 0x0000_007F_FFFF_FFFF)
//   0x1FF (256-511) — 内核半区  (0xFFFF_FF80_0000_0000 .. 0xFFFF_FFFF_FFFF_FFFF)

use core::ops::{Add, AddAssign, Sub, SubAssign};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::{PAGE_SHIFT, PAGE_SIZE};

/// Sv39 虚拟地址。
///
/// 保证规范形式：bits 63:39 全等于 bit 38。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(usize);

impl VirtAddr {
    /// 从原始 usize 构造虚拟地址，符号扩展到规范形式。
    ///
    /// 利用 bit 38 的值填充 bits 63:39（恒合法，不检查）。
    #[inline]
    pub const fn from_raw(addr: usize) -> Self {
        let sign = ((addr as isize) << (63 - 38)) >> (63 - 38);
        Self(sign as usize)
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

    /// 是否为用户地址（VPN[2] <= 255，即 bit 38 = 0）
    #[inline]
    pub fn is_user(self) -> bool {
        (self.0 >> 38) & 1 == 0
    }

    /// 是否为内核域地址：Sv39 高半区（bit 38 = 1），**或**内核镜像恒等区
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

/// 物理地址的原子包装 — 全局静态状态无锁读写用。
///
/// core 没有泛型 `Atomic<T>`，按本模块风格做具体包装（`PhysAddr` 为
/// `#[repr(transparent)]` 的 usize 新类型，原子性由内层 `AtomicUsize` 保证）。
#[repr(transparent)]
pub struct AtomicPhysAddr(AtomicUsize);

impl AtomicPhysAddr {
    /// 新建原子物理地址。
    #[inline]
    pub const fn new(pa: PhysAddr) -> Self {
        Self(AtomicUsize::new(pa.as_usize()))
    }

    /// 原子读取。
    #[inline]
    pub fn load(&self, order: Ordering) -> PhysAddr {
        PhysAddr::from_raw(self.0.load(order))
    }

    /// 原子写入。
    #[inline]
    pub fn store(&self, pa: PhysAddr, order: Ordering) {
        self.0.store(pa.as_usize(), order);
    }
}

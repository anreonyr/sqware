// 运行模式 — satp MODE 探测与几何派生。
//
// 全特性单一事实源：`MODE` 决定 层级数 / VA 宽度 / 分裂位，所有分页操作
// （walk 深度、VirtAddr 进位、satp 令牌、布局几何）从这里派生。
//
// 探测 = 写测（satp 为 WARL：不支持的模式被写入忽略）：对候选 57→48→39
// 逐一建立**候选层级**的最小恒等临时根（覆盖内核镜像与 ROOT 栈，取指可翻译），
// `satp::set(候选, 0, ppn)` → 回读 → 立即写回 Bare；回读命中即该硬件支持。
//
// 顺序不变量：MODE 未设时读侧一律兜底 Sv39——任何提前调用不依赖探测结果
// （低地址在各模式进位下恒等）。

use riscv::register::satp;

use crate::lock::OnceLock;
use crate::machine;
use crate::memory::PAGE_SHIFT;

use super::addr::VirtAddr;
use super::entry::PteFlags;
use super::table::TableNode;

/// 模式探测失败域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatpError {
    /// 三种 S 态分页模式均不被硬件支持。
    Unsupported,
    /// 探测临时根表帧耗尽（boot 级内存配置问题）。
    OutOfMemory,
}

/// 当前运行模式（探测成功后单次写入；未设 = 未探测）。
static MODE: OnceLock<satp::Mode> = OnceLock::new();

/// 探测硬件支持的 satp 模式并**部署为当前模式**（写 `MODE`）。
///
/// 逐候选（Sv57 → Sv48 → Sv39）写测回读，返回最高支持者；成功后 MODE 锁定，
/// 此后的 `mode()` / `geometry` / 布局几何按它派生。
///
/// # 前置
///
/// - satp 当前为 Bare（引导期，翻译未开）。
/// - 帧分配器已初始化（临时根表取帧）。
/// - SIE 关闭（探测期间无中断）。
///
/// @return：探测结果（已写入 MODE）。
///
/// # Errors
///
/// - [`SatpError::Unsupported`] — 三种模式全不支持（无 S 态分页，boot halt）。
/// - [`SatpError::OutOfMemory`] — 恒等临时根表帧耗尽。
pub fn detect() -> Result<satp::Mode, SatpError> {
    for candidate in [satp::Mode::Sv57, satp::Mode::Sv48, satp::Mode::Sv39] {
        if try_mode(candidate).is_ok() {
            MODE.set(candidate).expect("mode: detect is single-shot");
            return Ok(candidate);
        }
    }
    Err(SatpError::Unsupported)
}

/// 当前运行模式；未探测时兜底 Sv39（行为 = 现状）。
pub fn mode() -> satp::Mode {
    *MODE.get().unwrap_or(&satp::Mode::Sv39)
}

/// 模式几何 — 层级数 × VA 宽度（配对由穷尽 match 锁定，非法组合不可表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geo {
    /// 页表层级数（3/4/5）。
    pub levels: u8,
    /// 虚拟地址宽度（39/48/57）。
    pub va_bits: u8,
}

impl Geo {
    /// 分裂位 = 规范位 = W−1（用户/内核半区分界，即 `from_raw` 进位位）。
    #[inline]
    pub fn split_bit(self) -> u8 {
        self.va_bits - 1
    }
}

/// 模式 → 几何。
///
/// # Panics
///
/// `mode` 为 Bare/Sv64 等非分页模式时 panic（编码错误——探测只产出三分页模式）。
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

/// 当前模式的页表层级数（3/4/5）。
pub fn levels() -> usize {
    geometry(mode()).levels as usize
}

/// 内核半区起点（用户/内核分界）= `canonical(1 << split_bit)`。
///
/// Sv39: `0xFFFF_FFC0_0000_0000`· Sv48: `0xFFFF_8000_0000_0000`
/// · Sv57: `0xFFFF_FF00_0000_0000`。
pub fn lower() -> VirtAddr {
    VirtAddr::from_raw(1usize << geometry(mode()).split_bit())
}

/// 用户空间上界 = `1 << split_bit`（用户半区 `[0, upper)`）。
///
/// 与 [`lower()`]（内核空间下界，`canonical(1 << split_bit)`）同源自分裂位，
/// 两者之间是各模式的规范空洞（不可访问无人区）。**必须 `wrap` 纯位**——
/// `from_raw` 会把 `1 << split_bit`（bit split=1）进位成内核下界。
pub fn upper() -> VirtAddr {
    VirtAddr::wrap(1usize << geometry(mode()).split_bit())
}

/// 探测单个候选模式（写测回读，临时根表帧随 drop 归还）。
fn try_mode(candidate: satp::Mode) -> Result<(), SatpError> {
    let geo = geometry(candidate);
    let mut root = TableNode::root().map_err(|_| SatpError::OutOfMemory)?;
    // 恒等映射覆盖内核镜像 + ROOT 栈（探测码与栈都在其中，取指可翻译）。
    unsafe extern "C" {
        static _kernel_start: u8;
    }
    let range = (
        (&raw const _kernel_start).addr(),
        machine::root_stack_edge(),
    );
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
    // SAFETY: 恒等临时根已覆盖探测执行区；回读后立即写回 Bare。
    unsafe {
        satp::set(candidate, 0, ppn);
        core::arch::asm!("sfence.vma");
    }
    let got = satp::read().mode();
    // SAFETY: 立即关闭翻译，探测结束。
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

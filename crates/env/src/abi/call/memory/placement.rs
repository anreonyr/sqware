//! 一次选址的完整数据：选址方式与起始对齐。//!
//! 它**不是**内联寄存器载荷：`Attach` 只过一个指向它的只读用户指针，内核
//! 先 `copy_in` 快照这 [`Placement::WORDS`] 个字，再用本文件的 codec 解码并
//! checked 校验。非法字节永远不会被当成 Rust 枚举。
//!
//! 描述里只有选址数据：没有来源、授权、清理，也没有第二份长度（长度只来自
//! `ReferenceRange`）。区域固定为普通用户窗口（旧 `Mmap` 同口径）。

use crate::VirtAddr;
use crate::wire::Field;

use super::PAGE_SIZE;
use super::placement_at::PlacementAt;
use super::placement_error::PlacementError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    at: PlacementAt,
    align: usize,
}

impl Placement {
    /// 描述在用户内存里的定长字数；`encode` / `decode` 的数组长度从这里取。
    pub const WORDS: usize = 3;

    /// 窗口内最低的满足 `align` 的空档。
    pub fn lowest(align: usize) -> Option<Self> {
        Self::checked(PlacementAt::Lowest, align)
    }

    /// 窗口内最高的空档（页对齐）。
    pub fn highest() -> Self {
        Self {
            at: PlacementAt::Highest,
            align: PAGE_SIZE,
        }
    }

    /// 定点跨度：调用方已经知道地址。
    pub fn exact(at: VirtAddr, align: usize) -> Option<Self> {
        Self::checked(PlacementAt::Exact(at), align)
    }

    fn checked(at: PlacementAt, align: usize) -> Option<Self> {
        if !align.is_power_of_two() || align < PAGE_SIZE {
            return None;
        }
        if let PlacementAt::Exact(address) = at
            && address.get() % align != 0
        {
            return None;
        }
        Some(Self { at, align })
    }

    pub fn at(self) -> PlacementAt {
        self.at
    }

    pub fn align(self) -> usize {
        self.align
    }

    /// 把自身编码成描述的那几个字（写入方在栈上备好，再交给内核快照）。
    pub fn encode(self) -> [usize; Self::WORDS] {
        let (kind, address) = match self.at {
            PlacementAt::Lowest => (0, 0),
            PlacementAt::Highest => (1, 0),
            PlacementAt::Exact(address) => (2, address.get()),
        };
        [kind, address, self.align]
    }

    /// 由描述字解码；方式越界 → [`PlacementError::Tag`]，
    /// 方式认得而对齐/定点非法 → [`PlacementError::Align`]。
    pub fn decode(words: [usize; Self::WORDS]) -> Result<Self, PlacementError> {
        let [kind, address, align] = words;
        let at = match kind {
            0 if address == 0 => PlacementAt::Lowest,
            1 if address == 0 => PlacementAt::Highest,
            2 => PlacementAt::Exact(VirtAddr::new(address)),
            _ => return Err(PlacementError::Tag),
        };
        Self::checked(at, align).ok_or(PlacementError::Align)
    }

    pub fn decode_bytes(bytes: &[u8]) -> Result<Self, PlacementError> {
        Self::decode(super::fetch_words(bytes).ok_or(PlacementError::Tag)?)
    }
}

impl Field for Placement {
    const WIDTH: usize = Self::WORDS * u64::WIDTH;

    fn store(&self, out: &mut [u8]) {
        super::store_words(self.encode(), out);
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::decode_bytes(bytes).ok()
    }
}

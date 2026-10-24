//! 设备上下文（DC）：一个 requester id 的翻译上下文，64 字节扩展格式。
//!
//! 位取自 RISC-V IOMMU 规范 2.1；本景只开一级（S 阶段）翻译，二级是 Bare。

pub struct DeviceContext {
    words: [u64; 8],
}

/// `fsc.mode`：Sv39。
const FSC_MODE_SV39: u64 = 8;
/// `fsc.ppn`。
const FSC_PPN: u64 = (1u64 << 44) - 1;
/// `tc.V`：这一项有效。
const TC_VALID: u64 = 1;
pub const PSCID_LIMIT: u32 = 1 << 20;
pub const CAPACITY: usize = env::PAGE_SIZE / core::mem::size_of::<[u64; 8]>();

pub fn offset(requester: u16) -> Option<usize> {
    let bytes = core::mem::size_of::<[u64; 8]>();
    let offset = usize::from(requester) * bytes;
    (offset + bytes <= env::PAGE_SIZE).then_some(offset)
}

impl DeviceContext {
    /// 一级翻译的根表 `root`（物理页起址）。
    ///
    /// 不开 process id（`tc.PDTV` 不置 ⇒ process id 必须为 NOPROCID）、
    /// 不开二级（`iohgatp` = Bare）、不开 ATS 与 PRI。
    pub fn first_stage(root: usize, pscid: u32) -> Option<Self> {
        if !root.is_multiple_of(env::PAGE_SIZE) || (root as u64 >> 12) > FSC_PPN
            || pscid >= PSCID_LIMIT {
            return None;
        }
        let mut words = [0u64; 8];
        words[0] = TC_VALID;
        words[2] = u64::from(pscid) << 12;
        words[3] = (FSC_MODE_SV39 << 60) | ((root as u64 >> 12) & FSC_PPN);
        Some(Self { words })
    }

    pub fn words(&self) -> &[u64; 8] {
        &self.words
    }
}

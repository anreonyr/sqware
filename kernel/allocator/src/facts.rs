use crate::Region;

pub const MAX_RESERVED: usize = 2;

/// 机器给出的不可变内存事实。实现方每次从自己的唯一来源读取，不在这里保留副本。
#[derive(Clone, Copy, Debug)]
pub struct MachineFacts {
    pub free: Region,
    pub reserved: [Option<Region>; MAX_RESERVED],
}

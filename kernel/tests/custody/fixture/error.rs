// 最小 `FlatError` 夹具：这台台架只用到托管状态机会报的那几个。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatError {
    OutOfMemory,
    NotAligned,
    OutOfRange,
    Missing,
    Busy,
}

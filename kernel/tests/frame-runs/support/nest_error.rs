/// 帧分段与地址区间能返回的错误就这几个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestError {
    OutOfMemory,
    NotAligned,
    NoRegion,
    WidenDenied,
}

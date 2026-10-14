/// 帧分段能返回的错误就这两个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    NoRegion,
    WidenDenied,
}

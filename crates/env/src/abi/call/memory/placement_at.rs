//! 一次选址的方式：窗口内最低 / 最高空档，或调用方已定的那个地址。

use crate::VirtAddr;

/// 三者共用同一份占用检查：`Lowest` / `Highest` 在窗口内选空档，`Exact` 直接给出
/// 固定跨度——即使它落在所有窗口之外，也按同一全局占用登记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementAt {
    Lowest,
    Highest,
    Exact(VirtAddr),
}

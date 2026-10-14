use crate::core::memory::address::VirtAddr;

/// 选址方式。三者共用同一份占用检查：
/// `Lowest` / `Highest` 在窗口内选空档，`Exact` 直接给出固定跨度，
/// 即使它落在所有窗口之外，也按同一全局占用登记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlacementAt {
    Lowest,
    Highest,
    Exact(VirtAddr),
}

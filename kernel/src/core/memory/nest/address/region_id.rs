/// 地址区域：`Normal` / `Kernel` / `Resource` 各有一个动态选址窗口；
/// `Static` 只是地址记账政策：它没有动态窗口，跨度只按 `Exact` 登记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionId {
    Normal,
    Kernel,
    Resource,
    Static,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionId {
    Normal,
    Kernel,
    Resource,
}

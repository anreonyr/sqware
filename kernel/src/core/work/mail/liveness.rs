#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Liveness {
    Live,
    Dead,
}

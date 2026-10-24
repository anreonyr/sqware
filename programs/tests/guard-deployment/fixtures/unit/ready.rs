#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    OnSpawn,
    Signal(crate::Mark),
}

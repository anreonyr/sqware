#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Retirement {
    pub provider: &'static str,
    pub namespace: usize,
    pub face: &'static str,
}

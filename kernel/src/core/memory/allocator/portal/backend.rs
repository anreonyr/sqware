#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Backend {
    Bump = 1,
    Hybrid = 2,
    Spare = 3,
}

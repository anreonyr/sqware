#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Failure {
    Encode = 1,
    LockTimeout = 2,
    Create = 3,
    Write = 4,
}

impl Failure {
    pub(super) fn decode(value: usize) -> Option<Self> {
        match value {
            1 => Some(Self::Encode),
            2 => Some(Self::LockTimeout),
            3 => Some(Self::Create),
            4 => Some(Self::Write),
            _ => None,
        }
    }
}

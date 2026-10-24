#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 40)]
pub struct Supply {
    pub op: u8,
    pub hub: env::TaskId,
    pub serial: u64,
    pub source: env::PieToken,
    pub access: u32,
    pub back: env::PieToken,
}

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 64)]
pub struct Report {
    pub op: u8,
    pub serial: u64,
    pub want: u8,
    pub back: env::PieToken,
    pub auxiliary: env::TaskId,
    pub sensor: env::PieToken,
    pub iova: u64,
    pub bytes: u64,
}

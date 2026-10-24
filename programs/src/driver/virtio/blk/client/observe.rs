#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 32)]
pub struct Observe {
    pub op: u8,
    pub hub: env::TaskId,
    pub serial: u64,
    pub back: env::PieToken,
}

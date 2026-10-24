use env::PieToken;

pub struct Region {
    pub token: PieToken,
    pub memory: Box<[u64]>,
    pub mapped: bool,
}

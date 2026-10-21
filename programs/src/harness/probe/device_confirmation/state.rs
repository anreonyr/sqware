pub(super) struct State {
    pub party: u8,
    pub op: u8,
    pub target: &'static str,
    pub injection: env::PieToken,
    pub ack: Option<env::PieToken>,
    pub phase: u8,
}

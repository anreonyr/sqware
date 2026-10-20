/// Stable coalition creation request, scoped to the authenticated caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, env::Frame)]
pub struct FoundId {
    pub value: u64,
}

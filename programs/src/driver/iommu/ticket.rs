use super::handshake::Handshake;

/// One IOFENCE completion slot and value; the operation retains its deadline.
pub struct Ticket {
    pub(super) queue: usize,
    pub(super) slot: usize,
    pub(super) cookie: u32,
    pub(super) handshake: Option<Handshake>,
}

impl Ticket {
    pub(super) fn completion(&self) -> (*const u32, u32) {
        ((self.queue + self.slot) as *const u32, self.cookie)
    }
}

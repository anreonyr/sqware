use super::MAX_DESCRIPTION;

/// Bounded metadata delivered beside a resource already held by its recipient.
#[derive(Clone, Copy)]
pub struct SupplyFrame {
    pub length: u16,
    pub description: [u8; MAX_DESCRIPTION],
}

impl SupplyFrame {
    pub const LEN: usize = 2 + MAX_DESCRIPTION;
}

impl wire::Message for SupplyFrame {
    type In = Self;
    type Buf = [u8; 2 + MAX_DESCRIPTION];
    const EMPTY: Self::Buf = [0; 2 + MAX_DESCRIPTION];
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        if out.len() != Self::LEN || usize::from(self.length) > self.description.len() {
            return None;
        }
        out[..2].copy_from_slice(&self.length.to_le_bytes());
        out[2..].copy_from_slice(&self.description);
        Some(Self::LEN)
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::LEN {
            return None;
        }
        let length = u16::from_le_bytes(bytes[..2].try_into().ok()?);
        if usize::from(length) > MAX_DESCRIPTION {
            return None;
        }
        let mut description = [0; MAX_DESCRIPTION];
        description.copy_from_slice(&bytes[2..]);
        Some(Self {
            length,
            description,
        })
    }
}

//! Exact read-only snapshot of one function's first 256 configuration bytes.

use super::reader::{Reader, MAX_FUNCTIONS};

pub const LEN: usize = 256;

/// The bytes are captured once and never backed by a writable ECAM handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    requester: u16,
    bytes: [u8; LEN],
}

impl Snapshot {
    /// Reads exactly 256 bytes. If any byte is outside the mapped window the
    /// whole snapshot is absent.
    pub fn read(reader: &impl Reader, requester: u16) -> Option<Self> {
        if reader.limit() > MAX_FUNCTIONS || u32::from(requester) >= reader.limit() {
            return None;
        }
        let mut bytes = [0u8; LEN];
        for (register, byte) in bytes.iter_mut().enumerate() {
            *byte = reader.read8(requester, register as u16)?;
        }
        Some(Self { requester, bytes })
    }

    pub const fn requester(&self) -> u16 {
        self.requester
    }

    pub const fn bytes(&self) -> &[u8; LEN] {
        &self.bytes
    }

    pub fn vendor(&self) -> u16 {
        u16::from_le_bytes([self.bytes[0], self.bytes[1]])
    }

    pub fn device(&self) -> u16 {
        u16::from_le_bytes([self.bytes[2], self.bytes[3]])
    }

    /// 24-bit class code: base class, subclass, programming interface.
    pub fn class(&self) -> u32 {
        ((self.bytes[11] as u32) << 16)
            | ((self.bytes[10] as u32) << 8)
            | self.bytes[9] as u32
    }

    pub fn valid(&self) -> bool {
        !matches!(self.vendor(), 0 | 0xFFFF)
    }
}

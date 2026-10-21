//! Control frame for the opt-in kernel test fixture, not a driver protocol.

pub const TAG: &[u8; 8] = b"DMAFAIL1";
pub const LEN: usize = 56;
pub const REFERENCE_RELEASE: u64 = 1 << 8;

pub fn command(
    enabled: bool,
    from: usize,
    to: usize,
    op: u64,
    serial: u64,
    retainer: usize,
) -> [u8; LEN] {
    let mut bytes = [0u8; LEN];
    bytes[..8].copy_from_slice(TAG);
    bytes[8..16].copy_from_slice(&u64::from(enabled).to_le_bytes());
    bytes[16..24].copy_from_slice(&(from as u64).to_le_bytes());
    bytes[24..32].copy_from_slice(&(to as u64).to_le_bytes());
    bytes[32..40].copy_from_slice(&op.to_le_bytes());
    bytes[40..48].copy_from_slice(&serial.to_le_bytes());
    bytes[48..56].copy_from_slice(&(retainer as u64).to_le_bytes());
    bytes
}

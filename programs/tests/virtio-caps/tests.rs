use crate::{caps::Caps, registers::*};

fn cap(bytes: &mut [u8; 256], at: usize, next: u8, kind: u8, bar: u8, offset: u32, length: u32) {
    bytes[at] = CAP_VENDOR;
    bytes[at + 1] = next;
    bytes[at + 2] = if kind == CAP_NOTIFY { 20 } else { 16 };
    bytes[at + 3] = kind;
    bytes[at + 4] = bar;
    bytes[at + 8..at + 12].copy_from_slice(&offset.to_le_bytes());
    bytes[at + 12..at + 16].copy_from_slice(&length.to_le_bytes());
}

fn valid() -> [u8; 256] {
    let mut bytes = [0; 256];
    bytes[CONFIG_CAPABILITY] = 0x40;
    cap(&mut bytes, 0x40, 0x50, CAP_COMMON, 4, 0, 56);
    cap(&mut bytes, 0x50, 0x64, CAP_NOTIFY, 4, 0x1000, 2);
    bytes[0x60..0x64].copy_from_slice(&4u32.to_le_bytes());
    cap(&mut bytes, 0x64, 0, CAP_DEVICE, 4, 0x2000, 8);
    bytes
}

#[test]
fn region_length_is_independent_of_offset_and_other_region_lengths() {
    let (bar, caps) = Caps::read(&valid()).unwrap();
    assert_eq!(bar, 4);
    assert_eq!(caps.parts(), (0, 0x1000, 4, 0x2000));
    assert_eq!(Caps::from_parts(0, 0x1000, 4, 0x2000, 0x4000).unwrap(), caps);
}

#[test]
fn rejects_different_bars_and_reserved_bar_indices() {
    for bar in [5, 6, 0xFF] {
        let mut bytes = valid();
        bytes[0x68] = bar;
        assert_eq!(Caps::read(&bytes), Err("virtio: caps bar"));
    }
}

#[test]
fn rejects_truncated_known_capabilities_before_reading_extra_fields() {
    let mut bytes = valid();
    bytes[0x52] = 16;
    assert_eq!(Caps::read(&bytes), Err("virtio: caps length"));
    bytes[0x52] = 255;
    assert_eq!(Caps::read(&bytes), Err("virtio: caps length"));
    let mut bytes = valid();
    bytes[0x41] = 0xFC;
    bytes[0xFC] = CAP_VENDOR;
    bytes[0xFE] = 16;
    bytes[0xFF] = CAP_COMMON;
    assert_eq!(Caps::read(&bytes), Err("virtio: caps length"));
}

#[test]
fn a_short_non_vendor_capability_at_the_end_is_not_a_virtio_header() {
    let mut bytes = valid();
    bytes[0x65] = 0xFC;
    bytes[0xFC] = 0x05;
    assert!(Caps::read(&bytes).is_ok());
}

#[test]
fn rejects_loops_low_pointers_and_duplicate_regions() {
    let mut bytes = valid();
    bytes[0x65] = 0x40;
    assert_eq!(Caps::read(&bytes), Err("virtio: caps loop"));
    let mut bytes = valid();
    bytes[0x41] = 0x20;
    assert_eq!(Caps::read(&bytes), Err("virtio: caps header"));
    let mut bytes = valid();
    bytes[0x53] = CAP_COMMON;
    bytes[0x5C..0x60].copy_from_slice(&56u32.to_le_bytes());
    assert_eq!(Caps::read(&bytes), Err("virtio: caps duplicate"));
}

#[test]
fn rejects_short_regions_and_overflowing_region_end() {
    let mut bytes = valid();
    bytes[0x4C..0x50].copy_from_slice(&55u32.to_le_bytes());
    assert_eq!(Caps::read(&bytes), Err("virtio: caps region"));
    let mut bytes = valid();
    bytes[0x6C..0x70].copy_from_slice(&(u32::MAX - 3).to_le_bytes());
    assert_eq!(Caps::read(&bytes), Err("virtio: caps region"));
}

#[test]
fn mapped_bar_validation_covers_register_width_and_alignment() {
    assert!(Caps::from_parts(0x3FF0, 0x1000, 4, 0x2000, 0x4000).is_none());
    assert!(Caps::from_parts(0, 0x3FFF, 4, 0x2000, 0x4000).is_none());
    assert!(Caps::from_parts(0, 0x1000, 4, 0x3FFC, 0x4000).is_none());
    assert!(Caps::from_parts(1, 0x1000, 4, 0x2000, 0x4000).is_none());
}

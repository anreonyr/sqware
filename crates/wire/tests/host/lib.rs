#![allow(dead_code)]

extern crate alloc;
extern crate self as env;

// 仅模拟 env 的兼容重导出；codec 实现仍只有独立 wire 中的一份。
pub mod wire {
    pub use ::wire::*;
}

use alloc::string::String;
use mold::Frame;

#[derive(Debug, PartialEq, Eq, Frame)]
#[frame(codec = ::wire, len = 260)]
struct Packet {
    tag: u32,
    name: String,
}

#[derive(Debug, PartialEq, Eq, Frame)]
#[frame(len = 260)]
struct CompatiblePacket {
    tag: u32,
    name: String,
}

#[derive(Debug, PartialEq, Eq, Frame)]
#[frame(codec = ::wire)]
struct Items {
    count: u8,
    #[frame(count = count, fill = 0)]
    values: [u8; 3],
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::wire::{Field, Span};

    #[test]
    fn default_facade_and_explicit_codec_generate_identical_bytes() {
        let explicit = Packet { tag: 0x1234_5678, name: String::from("猫é") };
        let compatible = CompatiblePacket { tag: explicit.tag, name: explicit.name.clone() };
        let mut left = [0; Packet::LEN];
        let mut right = [0; CompatiblePacket::LEN];
        let end = explicit.store_at(&mut left, 0).unwrap();
        assert_eq!(compatible.store_at(&mut right, 0), Some(end));
        assert_eq!(left, right);
        assert_eq!(CompatiblePacket::fetch_at(&left[..end], 0), Some((compatible, end)));
        assert_eq!(Packet::fetch_at(&right[..end], 0), Some((explicit, end)));
    }

    #[test]
    fn field_width_endianness_and_bounds_match_the_wire_contract() {
        let mut bytes = [0u8; 4];
        0x1234_5678u32.store(&mut bytes);
        assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12]);
        assert_eq!(u32::fetch(&bytes), Some(0x1234_5678));
        assert_eq!(u32::fetch(&bytes[..3]), None);
        assert_eq!(bool::fetch(&[2]), None);
    }

    #[test]
    fn derived_variable_utf8_frame_round_trips_bytes_and_rejects_truncation() {
        let packet = Packet { tag: 0x1234_5678, name: String::from("猫é") };
        let mut bytes = [0u8; Packet::LEN];
        let end = packet.store_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes[..end], &[0x78, 0x56, 0x34, 0x12, 5, 0xe7, 0x8c, 0xab, 0xc3, 0xa9]);
        let (decoded, end2) = Packet::fetch_at(&bytes[..end], 0).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(end2, end);
        assert_eq!(Packet::fetch_at(&bytes[..end - 1], 0), None);
        assert_eq!(String::fetch_at(&[2, 0xc3, 0x28], 0), None);
        assert_eq!(String::from("x").store_at(&mut [0], usize::MAX), None);
        assert_eq!(String::fetch_at(&[0], usize::MAX), None);
        assert_eq!(::wire::store_bytes(&mut [0], usize::MAX, &[1]), None);
    }

    #[test]
    fn string_length_byte_accepts_255_bytes_and_rejects_256() {
        let max = String::from("é".repeat(127) + "x");
        assert_eq!(max.len(), 255);
        let mut bytes = alloc::vec![0; 256];
        assert_eq!(max.store_at(&mut bytes, 0), Some(256));
        let (decoded, end) = String::fetch_at(&bytes, 0).unwrap();
        assert_eq!(decoded, max);
        assert_eq!(end, 256);
        let too_long = String::from("a".repeat(256));
        assert_eq!(too_long.store_at(&mut bytes, 0), None);
    }

    #[test]
    fn frame_count_bounds_and_length_arithmetic_overflow_are_rejected() {
        let invalid = Items { count: 4, values: [1, 2, 3] };
        let mut bytes = [0; Items::LEN];
        assert_eq!(invalid.store_at(&mut bytes, 0), None);
        assert_eq!(Items::fetch_at(&[4, 1, 2, 3], 0), None);
        assert_eq!(::wire::total(&[Some(usize::MAX), Some(1)]), None);
        assert_eq!(::wire::times(usize::MAX, Some(2)), None);
    }
}

//! Regression tests for the production canonical input mode.
#![allow(dead_code)]
#[path = "../../src/user/terminal/core/mod.rs"]
mod core;

#[cfg(test)]
mod tests {
    use super::core::{canonical::{Canonical, LINE_MAX}, mode::Input};
    fn line(input: Input<'_>) -> Vec<u8> {
        match input { Input::Line(bytes) => bytes.to_vec(), _ => panic!("expected a line") }
    }
    #[test]
    fn canonical_delivers_exit_as_data_and_folds_crlf() {
        let mut mode = Canonical::new();
        for &b in b"exit" { mode.feed(b); }
        assert_eq!(line(mode.feed(b'\r').input), b"exit");
        let folded = mode.feed(b'\n');
        assert!(folded.echo().is_empty());
        assert!(matches!(folded.input, Input::More));
        mode.feed(b'x');
        assert_eq!(line(mode.feed(4).input), b"x");
        assert!(matches!(mode.feed(4).input, Input::Eof));
    }

    #[test]
    fn clearing_secret_preserves_crlf_folding() {
        let mut mode = Canonical::new();
        for &b in b"secret" { mode.feed(b); }
        assert_eq!(line(mode.feed(b'\r').input), b"secret");
        mode.clear();
        assert!(matches!(mode.feed(b'\n').input, Input::More));
        assert!(line(mode.feed(b'\n').input).is_empty());
    }

    #[test]
    fn editing_preserves_utf8_and_erases_control_echo() {
        let mut mode = Canonical::new();
        for &b in "a中".as_bytes() { mode.feed(b); }
        assert_eq!(mode.feed(0x7f).echo(), b"\x08 \x08");
        assert_eq!(line(mode.feed(b'\n').input), b"a");
        assert_eq!(mode.feed(3).echo(), b"^C");
        assert_eq!(mode.feed(8).echo(), b"\x08\x08  \x08\x08");
        mode.feed(b'x');
        assert_eq!(mode.feed(0x15).echo(), b"^U\r\n");
        assert!(line(mode.feed(b'\n').input).is_empty());
    }

    #[test]
    fn full_line_rejects_unstored_echo_and_remains_editable() {
        let mut mode = Canonical::new();
        for _ in 0..LINE_MAX { mode.feed(b'a'); }
        assert!(mode.feed(b'b').echo().is_empty());
        mode.feed(8);
        mode.feed(b'c');
        let got = line(mode.feed(b'\n').input);
        assert_eq!(got.len(), LINE_MAX);
        assert_eq!(got[LINE_MAX - 1], b'c');
    }


    #[test]
    fn eof_is_an_event_and_does_not_end_the_mode() {
        let mut mode = Canonical::new();
        assert!(matches!(mode.feed(4).input, Input::Eof));
        for &b in b"next" { mode.feed(b); }
        assert_eq!(line(mode.feed(b'\n').input), b"next");
    }
    #[test]
    fn gap_reset_discards_partial_input_and_crlf_state() {
        let mut mode = Canonical::new();
        mode.feed(b'x');
        mode.reset();
        assert!(matches!(mode.feed(4).input, Input::Eof));
        mode.feed(b'\r');
        mode.reset();
        assert!(line(mode.feed(b'\n').input).is_empty());
    }
}

#[cfg(test)]
#[path = "../../../crates/protocol/src/wire/message.rs"]
pub mod message;
#[cfg(test)]
mod wire { pub use crate::message; }
#[cfg(test)]
#[path = "../../../crates/protocol/src/service/terminal/frame.rs"]
mod frame;

#[cfg(test)]
mod stream_tests {
    use super::{frame, message::Message};

    #[test]
    fn eof_is_distinct_from_a_line_and_preserves_binary_data() {
        let line = frame::Input::data(b"a\0b\n").unwrap();
        let mut bytes = [0; frame::Input::LEN];
        let n = line.store(&mut bytes).unwrap();
        let decoded = frame::Input::fetch(&bytes[..n]).unwrap();
        assert_eq!(decoded.kind, frame::DATA);
        assert_eq!(decoded.bytes(), b"a\0b\n");
        let n = frame::Input::eof().store(&mut bytes).unwrap();
        let decoded = frame::Input::fetch(&bytes[..n]).unwrap();
        assert_eq!(decoded.kind, frame::EOF);
        assert!(decoded.bytes().is_empty());
        assert!(frame::Input::data(&[]).is_none());
    }
    #[test]
    fn malformed_or_truncated_input_frames_are_rejected() {
        let mut bytes = [0; frame::Input::LEN];
        let n = frame::Input::data(b"\n").unwrap().store(&mut bytes).unwrap();
        assert!(frame::Input::fetch(&bytes[..n - 1]).is_none());
        bytes[0] = frame::EOF;
        assert!(frame::Input::fetch(&bytes[..n]).is_none());
        bytes[0] = 255;
        assert!(frame::Input::fetch(&bytes[..n]).is_none());
        bytes[0] = frame::DATA;
        bytes[1..5].copy_from_slice(&((frame::MAX + 1) as u32).to_le_bytes());
        assert!(frame::Input::fetch(&bytes).is_none());
    }
    #[test]
    fn input_size_is_bounded_independently_of_uart_frames() {
        assert!(frame::Input::data(&[b'x'; frame::MAX]).is_some());
        assert!(frame::Input::data(&[b'x'; frame::MAX + 1]).is_none());
    }
}

//! Canonical input: line buffering, CRLF folding, echo, erase, kill and EOF.

use super::mode::{Effect, Input};

/// Maximum buffered line length in bytes. Further input is discarded without echo.
pub const LINE_MAX: usize = 4096;

pub struct Canonical {
    raw: [u8; LINE_MAX],
    n: usize,
    cr: bool,
}

impl Canonical {
    pub fn new() -> Self {
        Self { raw: [0; LINE_MAX], n: 0, cr: false }
    }

    pub fn clear(&mut self) {
        self.raw.fill(0);
        self.n = 0;
    }

    pub fn reset(&mut self) {
        self.clear();
        self.cr = false;
    }

    pub fn feed(&mut self, b: u8) -> Effect<'_> {
        if b == b'\n' && self.cr {
            self.cr = false;
            return Effect::new(&[], Input::More);
        }
        self.cr = b == b'\r';
        match b {
            b'\r' | b'\n' => {
                let n = self.n;
                self.n = 0;
                Effect::new(b"\r\n", Input::Line(&self.raw[..n]))
            }
            0x7f | 0x08 => {
                let columns = self.erase();
                // ECHOCTL characters occupy two columns; other characters erase one.
                let echo: &[u8] = match columns {
                    0 => b"",
                    2 => b"\x08\x08  \x08\x08",
                    _ => b"\x08 \x08",
                };
                Effect::new(echo, Input::More)
            }
            0x15 => {
                let echo: &[u8] = if self.n > 0 { b"^U\r\n" } else { b"" };
                self.n = 0;
                Effect::new(echo, Input::More)
            }
            0x04 => {
                let n = self.n;
                self.n = 0;
                let input = if n == 0 { Input::Eof } else { Input::Line(&self.raw[..n]) };
                Effect::new(b"^D", input)
            }
            _ => {
                let Some(slot) = self.raw.get_mut(self.n) else {
                    return Effect::new(&[], Input::More);
                };
                *slot = b;
                self.n += 1;
                if b < 0x20 {
                    Effect::new(&[b'^', b + 0x40], Input::More)
                } else {
                    Effect::new(&[b], Input::More)
                }
            }
        }
    }

    /// Removes a UTF-8 byte sequence. Wide and combining glyphs are not measured.
    fn erase(&mut self) -> usize {
        if self.n == 0 { return 0; }
        let mut k = self.n - 1;
        while k > 0 && (self.raw[k] & 0xC0) == 0x80 { k -= 1; }
        self.n = k;
        if self.raw[k] < 0x20 { 2 } else { 1 }
    }
}

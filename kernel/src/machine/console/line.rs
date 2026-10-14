use ::core::fmt::{self, Write};

const LINE_MAX: usize = 256;

pub(super) struct Line {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl Line {
    pub(super) fn new() -> Self {
        Line {
            buf: [0u8; LINE_MAX],
            len: 0,
        }
    }

    pub(super) fn emit(&mut self) {
        if self.len == 0 {
            return;
        }
        if let Ok(text) = ::core::str::from_utf8(&self.buf[..self.len]) {
            super::write(text);
        }
        self.len = 0;
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.buf.len() - self.len >= s.len() {
            self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
        } else {
            self.emit();
            super::write(s);
        }
        Ok(())
    }
}

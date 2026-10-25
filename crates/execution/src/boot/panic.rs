//! Allocation-free panic report for freestanding program ELFs.

use core::fmt::{self, Write};
use core::panic::PanicInfo;

use env::NOTE_MAX;

/// Panic text is assembled on the stack; formatting here must never allocate.
struct Note {
    buf: [u8; NOTE_MAX],
    len: usize,
}

impl Note {
    const fn new() -> Self {
        Self {
            buf: [0; NOTE_MAX],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("<note>")
    }
}

impl fmt::Write for Note {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut count = text.len().min(self.buf.len() - self.len);
        while count > 0 && !text.is_char_boundary(count) {
            count -= 1;
        }
        self.buf[self.len..self.len + count].copy_from_slice(&text.as_bytes()[..count]);
        self.len += count;
        Ok(())
    }
}

#[cfg(target_arch = "riscv64")]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut note = Note::new();
    let _ = write!(note, "{}", info.message());
    if let Some(at) = info.location() {
        let _ = write!(note, " at {}:{}:{}", at.file(), at.line(), at.column());
    }
    crate::room::reap(env::EXIT_PANIC, Some(note.as_str()))
}

use ::core::fmt::{self, Write};

use super::{BootError, HartId, InitError};

struct Buffer {
    bytes: [u8; 2048],
    len: usize,
}

impl Buffer {
    fn new() -> Self {
        Self {
            bytes: [0; 2048],
            len: 0,
        }
    }
    fn text(&self) -> &str {
        ::core::str::from_utf8(&self.bytes[..self.len]).unwrap()
    }
}

impl Write for Buffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
        let dest = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        dest.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

pub fn accept() {
    let mut output = Buffer::new();
    {
        let _guard = crate::core::memory::allocator::NoAllocation::enter();
        BootError::Allocator(erra::Error::new(
            "initializing allocator",
            InitError::OutOfMemory,
        ))
        .print(&mut output);
    }
    assert_eq!(
        output.text(),
        concat!(
            "sqware: boot failed\n\n",
            "  operation  initialize allocator\n",
            "  context    initializing allocator\n",
            "  reason     memory allocation failed while initializing allocator\n\n",
        )
    );

    let error = BootError::HartStart {
        hart: HartId::new(2),
        source: erra::Error::new("s-mode environment call", sbi::ecall::SError::Denied),
    };
    let mut output = Buffer::new();
    {
        let _guard = crate::core::memory::allocator::NoAllocation::enter();
        error.print(&mut output);
    }
    assert_eq!(
        output.text(),
        concat!(
            "sqware: boot failed\n\n",
            "  operation  start hart\n",
            "  hart       2\n",
            "  context    s-mode environment call\n",
            "  reason     access denied\n\n",
        )
    );
}

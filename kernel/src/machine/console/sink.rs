use ::core::fmt::{self, Write};

pub struct Sink;

impl Write for Sink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        super::_write(format_args!("{s}"));
        Ok(())
    }
}

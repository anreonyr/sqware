//! Input interpretation and its bounded echo effect.

use super::canonical::Canonical;

pub enum Input<'a> {
    More,
    Line(&'a [u8]),
    Eof,
}

/// Largest echo from one input byte: erasing a two-column control character.
pub const ECHO_MAX: usize = 6;

pub struct Effect<'a> {
    echo: [u8; ECHO_MAX],
    n: usize,
    pub input: Input<'a>,
}

impl<'a> Effect<'a> {
    pub(super) fn new(echo: &[u8], input: Input<'a>) -> Self {
        let mut bytes = [0; ECHO_MAX];
        bytes[..echo.len()].copy_from_slice(echo);
        Self { echo: bytes, n: echo.len(), input }
    }

    pub fn echo(&self) -> &[u8] { &self.echo[..self.n] }
}

pub enum Mode {
    Canonical(Canonical),
}

impl Mode {
    pub fn canonical() -> Self { Self::Canonical(Canonical::new()) }

    pub fn clear(&mut self) {
        match self { Self::Canonical(mode) => mode.clear() }
    }

    pub fn reset(&mut self) {
        match self { Self::Canonical(mode) => mode.reset() }
    }

    pub fn feed(&mut self, b: u8) -> Effect<'_> {
        match self { Self::Canonical(mode) => mode.feed(b) }
    }
}

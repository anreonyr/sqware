use crate::{Source, Span};
use alloc::{format, string::String, vec::Vec};
use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Lexical,
    Syntax,
    Unbound,
    Arity,
    Type,
    Arithmetic,
    ResourceLimit,
    Cancelled,
    Host,
    Busy,
    StaleCall,
    StaleValue,
}
#[derive(Clone, Debug)]
pub struct TraceFrame {
    pub source: Source,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub kind: ErrorKind,
    pub message: String,
    pub source: Source,
    pub span: Span,
    pub trace: Vec<TraceFrame>,
}
impl Diagnostic {
    pub(crate) fn new(
        kind: ErrorKind,
        message: impl Into<String>,
        source: &Source,
        span: Span,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: source.clone(),
            span,
            trace: Vec::new(),
        }
    }
    pub fn render(&self) -> String {
        format!("{self}")
    }
}
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (line, column) = self.source.location(self.span.start);
        write!(
            f,
            "{}:{line}:{column}: {:?}: {}",
            self.source.name(),
            self.kind,
            self.message
        )?;
        for frame in self.trace.iter().take(16) {
            let (line, column) = frame.source.location(frame.span.start);
            write!(f, "\n  at {}:{line}:{column}", frame.source.name())?;
        }
        Ok(())
    }
}

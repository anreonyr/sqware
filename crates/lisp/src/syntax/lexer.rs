use crate::{Diagnostic, ErrorKind, Source, Span};
use alloc::string::String;
use logos::Logos;

#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
#[logos(skip(r";[^\n]*", allow_greedy = true))]
pub(crate) enum Token {
    #[token("(")]
    Open,
    #[token(")")]
    Shut,
    #[token("'")]
    Quote,
    #[token(".", priority = 3)]
    Dot,
    #[token("#t", priority = 3)]
    True,
    #[token("#f", priority = 3)]
    False,
    #[token("#u8(")]
    Bytes,
    #[regex(r"[+-]?[0-9]+", priority = 5)]
    Integer,
    #[token("\"", quoted)]
    String(bool),
    #[regex(r#"[^\s()'";]+"#)]
    Symbol,
}
pub(crate) fn scan(source: &str) -> impl Iterator<Item = (Result<Token, ()>, Span)> + '_ {
    Token::lexer(source)
        .spanned()
        .map(|(token, range)| (token, Span::new(range.start, range.end)))
}
pub(crate) fn string(source: &Source, span: Span) -> Result<String, Diagnostic> {
    let raw = &source.text()[span.start + 1..span.end - 1];
    let mut chars = raw.chars();
    let mut out = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        out.push(match chars.next() {
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('"') => '"',
            Some('\\') => '\\',
            _ => {
                return Err(Diagnostic::new(
                    ErrorKind::Lexical,
                    "invalid string escape",
                    source,
                    span,
                ));
            }
        });
    }
    Ok(out)
}

fn quoted(lexer: &mut logos::Lexer<'_, Token>) -> bool {
    let mut escaped = false;
    for (index, byte) in lexer.remainder().bytes().enumerate() {
        if !escaped && byte == b'"' {
            lexer.bump(index + 1);
            return true;
        }
        if escaped {
            escaped = false;
        } else {
            escaped = byte == b'\\';
        }
    }
    lexer.bump(lexer.remainder().len());
    false
}
pub(crate) fn unfinished(source: &Source, span: Span) -> Result<(), Diagnostic> {
    let mut chars = source.text()[span.start + 1..span.end].chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'r' | 't' | '"' | '\\') | None => {}
                _ => {
                    return Err(Diagnostic::new(
                        ErrorKind::Lexical,
                        "invalid string escape",
                        source,
                        span,
                    ));
                }
            }
        }
    }
    Ok(())
}

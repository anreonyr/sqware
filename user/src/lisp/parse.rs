//! 词法/语法解析器（一行字节 → 表达式树）。

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::core::Core;
use super::kernel::{LispError, Val};

pub(super) struct Parser<'a> {
    pub(super) core: &'a mut Core,
    pub(super) line: &'a [u8],
    pub(super) pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.line.len() && matches!(self.line[self.pos], b' ' | b'\t') {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.line.get(self.pos).copied()
    }

    fn form(&mut self) -> Result<Val, LispError> {
        self.skip_ws();
        match self.peek() {
            Some(b'(') => self.list(),
            Some(b')') => Err(LispError::Parse),
            Some(_) => self.atom(),
            None => Err(LispError::Parse),
        }
    }

    fn list(&mut self) -> Result<Val, LispError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(LispError::Parse),
                Some(b')') => {
                    self.pos += 1;
                    break;
                }
                Some(_) => items.push(self.form()?),
            }
        }
        let mut v = Val::Nil;
        for item in items.into_iter().rev() {
            v = Val::Cons(Box::new((item, v)));
        }
        Ok(v)
    }

    fn atom(&mut self) -> Result<Val, LispError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'(' | b')') {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(LispError::Parse);
        }
        let tok = &self.line[start..self.pos];
        if is_number(tok) {
            let text = core::str::from_utf8(tok).map_err(|_| LispError::Parse)?;
            let n = text.parse::<i64>().map_err(|_| LispError::Parse)?;
            Ok(Val::Int(n))
        } else {
            Ok(Val::Sym(self.core.intern(tok)))
        }
    }
}

fn is_number(tok: &[u8]) -> bool {
    let mut it = tok.iter();
    match it.next() {
        Some(b'-') => match it.next() {
            Some(d) if d.is_ascii_digit() => it.all(u8::is_ascii_digit),
            _ => false,
        },
        Some(d) if d.is_ascii_digit() => it.all(u8::is_ascii_digit),
        _ => false,
    }
}

impl Core {
    pub fn read(&mut self, line: &[u8]) -> Result<Val, LispError> {
        let mut p = Parser { core: self, line, pos: 0 };
        let v = p.form()?;
        p.skip_ws();
        if p.pos != p.line.len() {
            return Err(LispError::Parse);
        }
        Ok(v)
    }
}
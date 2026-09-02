//! 语言内核状态（read/eval/print 共享可变状态）+ symbol 表与全局/调用帧。

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::kernel::{BUILTINS, LispError, Sym, Val};

pub struct Core {
    pub(crate) syms: Vec<Vec<u8>>,
    pub(crate) globals: Vec<(Sym, Rc<Val>)>,
    pub(crate) frames: Vec<Vec<(Sym, Rc<Val>)>>,
}

impl Core {
    pub fn new() -> Core {
        let mut core = Core {
            syms: Vec::new(),
            globals: Vec::new(),
            frames: Vec::new(),
        };
        for name in BUILTINS {
            core.intern(name.as_bytes());
        }
        core
    }

    pub fn intern(&mut self, name: &[u8]) -> Sym {
        for (i, n) in self.syms.iter().enumerate() {
            if n.as_slice() == name {
                return Sym(i);
            }
        }
        self.syms.push(name.to_vec());
        Sym(self.syms.len() - 1)
    }

    pub(crate) fn bi(&self, name: &str) -> Sym {
        for (i, n) in self.syms.iter().enumerate() {
            if n.as_slice() == name.as_bytes() {
                return Sym(i);
            }
        }
        unreachable!("builtin not interned: {name}")
    }

    pub(crate) fn lookup(&self, s: Sym) -> Result<Rc<Val>, LispError> {
        for frame in self.frames.iter().rev() {
            for (k, v) in frame {
                if *k == s {
                    return Ok(v.clone());
                }
            }
        }
        for (k, v) in &self.globals {
            if *k == s {
                return Ok(v.clone());
            }
        }
        if s == self.bi("t") {
            return Ok(Rc::new(Val::T));
        }
        if s == self.bi("nil") {
            return Ok(Rc::new(Val::Nil));
        }
        if self.is_builtin(s) {
            return Ok(Rc::new(Val::Sym(s)));
        }
        Err(LispError::Unbound)
    }

    pub(crate) fn is_builtin(&self, s: Sym) -> bool {
        for n in BUILTINS {
            if *n != "t" && *n != "nil" && s == self.bi(n) {
                return true;
            }
        }
        false
    }

    pub fn print(&self, val: &Val) -> String {
        match val {
            Val::Int(n) => format!("{n}"),
            Val::Sym(s) => String::from_utf8_lossy(&self.syms[s.0]).into_owned(),
            Val::T => "t".into(),
            Val::Nil => "()".into(),
            Val::Fn(_) => "#<fn>".into(),
            Val::Cons(b) => {
                let mut s = String::from("(");
                let mut cur = b;
                loop {
                    s.push_str(&self.print(&cur.0));
                    match &cur.1 {
                        Val::Cons(next) => {
                            s.push(' ');
                            cur = next;
                        }
                        Val::Nil => break,
                        tail => {
                            s.push_str(" . ");
                            s.push_str(&self.print(tail));
                            break;
                        }
                    }
                }
                s.push(')');
                s
            }
        }
    }
}

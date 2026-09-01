//! 教学 Lisp 语言内核 + REPL 适配。

extern crate alloc;

mod kernel;
mod core;
mod parse;
mod vm;
mod repl;

pub use kernel::{FnDef, LispError, Sym, Val};
pub use core::Core;
pub use repl::repl;
//! 教学 Lisp 语言内核 + REPL 适配。

extern crate alloc;

mod core;
mod kernel;
mod parse;
mod repl;
mod vm;

pub use core::Core;
pub use kernel::{FnDef, LispError, Sym, Val};
pub use repl::repl;

//! 值类型与错误域。

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LispError {
    Parse,
    Unbound,
    Arity,
    BadForm,
    NotCallable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sym(pub usize);

#[derive(Debug, Clone)]
pub struct FnDef {
    pub params: Vec<Sym>,
    pub body: Rc<Val>,
}

#[derive(Debug, Clone)]
pub enum Val {
    Int(i64),
    Sym(Sym),
    Nil,
    T,
    Cons(Box<(Val, Val)>),
    Fn(Box<FnDef>),
}

/// 预登记名单（构造即 intern；id 即登记序）。
pub(super) const BUILTINS: &[&str] = &[
    "t", "nil", "quote", "car", "cdr", "cons", "eq?", "+", "-", "*", "/", "if", "define", "lambda",
    "exit",
];
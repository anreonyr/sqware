//! 求值器：eval / apply / 特殊形式 / 内建分派。

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;

use super::core::Core;
use super::kernel::{FnDef, LispError, Val};

impl Core {
    pub fn eval(&mut self, val: Val) -> Result<Val, LispError> {
        self.eval_rc(&val)
    }

    fn eval_rc(&mut self, v: &Val) -> Result<Val, LispError> {
        match v {
            Val::Int(n) => Ok(Val::Int(*n)),
            Val::T => Ok(Val::T),
            Val::Nil => Ok(Val::Nil),
            Val::Fn(def) => Ok(Val::Fn(def.clone())),
            Val::Sym(s) => {
                if self.is_builtin(*s) {
                    return Ok(Val::Sym(*s));
                }
                let r = self.lookup(*s)?;
                self.eval_rc(&r)
            }
            Val::Cons(_) => self.apply(v),
        }
    }

    fn apply(&mut self, v: &Val) -> Result<Val, LispError> {
        let Val::Cons(b) = v else {
            unreachable!("eval_rc guaranteed Cons")
        };
        let (head, rest) = (&b.0, &b.1);
        if let Val::Sym(s) = head {
            if *s == self.bi("quote") {
                return self.sp_quote(rest);
            }
            if *s == self.bi("if") {
                return self.sp_if(rest);
            }
            if *s == self.bi("define") {
                return self.sp_define(rest);
            }
            if *s == self.bi("lambda") {
                return self.sp_lambda(rest);
            }
        }
        let f = self.eval_rc(head)?;
        let mut args = Vec::new();
        let mut cur = rest;
        loop {
            match cur {
                Val::Cons(b) => {
                    args.push(self.eval_rc(&b.0)?);
                    cur = &b.1;
                }
                Val::Nil => break,
                _ => return Err(LispError::BadForm),
            }
        }
        self.apply_fn(f, args)
    }

    fn sp_quote(&mut self, rest: &Val) -> Result<Val, LispError> {
        match rest {
            Val::Cons(b) if matches!(b.1, Val::Nil) => Ok(b.0.clone()),
            _ => Err(LispError::Arity),
        }
    }

    fn sp_if(&mut self, rest: &Val) -> Result<Val, LispError> {
        let (c, r1) = take_ref(rest)?;
        let (a, r2) = take_ref(r1)?;
        let (b, r3) = take_ref(r2)?;
        if !matches!(r3, Val::Nil) {
            return Err(LispError::Arity);
        }
        let cond = self.eval_rc(c)?;
        if matches!(cond, Val::Nil) {
            self.eval_rc(b)
        } else {
            self.eval_rc(a)
        }
    }

    fn sp_define(&mut self, rest: &Val) -> Result<Val, LispError> {
        let (name, r1) = take_ref(rest)?;
        let (expr, r2) = take_ref(r1)?;
        if !matches!(r2, Val::Nil) {
            return Err(LispError::Arity);
        }
        let Val::Sym(s) = name else {
            return Err(LispError::BadForm);
        };
        let v = self.eval_rc(expr)?;
        if let Some(slot) = self.globals.iter_mut().find(|(k, _)| *k == *s) {
            slot.1 = Rc::new(v);
        } else {
            self.globals.push((*s, Rc::new(v)));
        }
        Ok(Val::Nil)
    }

    fn sp_lambda(&mut self, rest: &Val) -> Result<Val, LispError> {
        let (params, r1) = take_ref(rest)?;
        let (body, r2) = take_ref(r1)?;
        if !matches!(r2, Val::Nil) {
            return Err(LispError::BadForm);
        }
        let mut ps = Vec::new();
        let mut cur = params;
        while let Val::Cons(b) = cur {
            let Val::Sym(s) = &b.0 else {
                return Err(LispError::BadForm);
            };
            ps.push(*s);
            cur = &b.1;
        }
        if !matches!(cur, Val::Nil) {
            return Err(LispError::BadForm);
        }
        Ok(Val::Fn(Box::new(FnDef {
            params: ps,
            body: Rc::new(body.clone()),
        })))
    }

    fn apply_fn(&mut self, f: Val, args: Vec<Val>) -> Result<Val, LispError> {
        match f {
            Val::Fn(def) => {
                if def.params.len() != args.len() {
                    return Err(LispError::Arity);
                }
                self.frames.push(
                    def.params
                        .iter()
                        .cloned()
                        .zip(args.into_iter().map(Rc::new))
                        .collect(),
                );
                let r = self.eval_rc(&def.body);
                self.frames.pop();
                r
            }
            Val::Sym(s) => {
                let n = sym_name(&self.syms[s.0]);
                match n {
                    "car" => {
                        let v = one(&args)?;
                        match v {
                            Val::Cons(b) => Ok(b.0.clone()),
                            _ => Err(LispError::BadForm),
                        }
                    }
                    "cdr" => {
                        let v = one(&args)?;
                        match v {
                            Val::Cons(b) => Ok(b.1.clone()),
                            _ => Err(LispError::BadForm),
                        }
                    }
                    "cons" => {
                        let (a, b) = two(&args)?;
                        Ok(Val::Cons(Box::new((a.clone(), b.clone()))))
                    }
                    "eq?" => {
                        let (a, b) = two(&args)?;
                        Ok(if lisp_eq(a, b) { Val::T } else { Val::Nil })
                    }
                    "+" => fold(&args, 0i64, |a, x| a.wrapping_add(x)),
                    "-" => {
                        let mut it = args.iter();
                        let first = int(it.next())?;
                        if args.len() == 1 {
                            return Ok(Val::Int(-first));
                        }
                        let mut acc = first;
                        for x in it {
                            acc = acc.wrapping_sub(int(Some(x))?);
                        }
                        Ok(Val::Int(acc))
                    }
                    "*" => fold(&args, 1i64, |a, x| a.wrapping_mul(x)),
                    "/" => {
                        let mut it = args.iter();
                        let first = int(it.next())?;
                        let mut acc = first;
                        for x in it {
                            let d = int(Some(x))?;
                            if d == 0 {
                                return Err(LispError::BadForm);
                            }
                            acc = acc.wrapping_div(d);
                        }
                        Ok(Val::Int(acc))
                    }
                    _ => Err(LispError::NotCallable),
                }
            }
            _ => Err(LispError::NotCallable),
        }
    }
}

fn take_ref(v: &Val) -> Result<(&Val, &Val), LispError> {
    match v {
        Val::Cons(b) => Ok((&b.0, &b.1)),
        _ => Err(LispError::BadForm),
    }
}

fn one<'a>(args: &'a [Val]) -> Result<&'a Val, LispError> {
    if args.len() == 1 {
        Ok(&args[0])
    } else {
        Err(LispError::Arity)
    }
}

fn two<'a>(args: &'a [Val]) -> Result<(&'a Val, &'a Val), LispError> {
    if args.len() == 2 {
        Ok((&args[0], &args[1]))
    } else {
        Err(LispError::Arity)
    }
}

fn int(v: Option<&Val>) -> Result<i64, LispError> {
    match v {
        Some(Val::Int(n)) => Ok(*n),
        _ => Err(LispError::BadForm),
    }
}

fn lisp_eq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Int(x), Val::Int(y)) => x == y,
        (Val::Sym(x), Val::Sym(y)) => x == y,
        _ => false,
    }
}

fn fold(args: &[Val], init: i64, op: impl Fn(i64, i64) -> i64) -> Result<Val, LispError> {
    if args.is_empty() {
        return Err(LispError::Arity);
    }
    let mut acc = init;
    for a in args {
        acc = op(acc, int(Some(a))?);
    }
    Ok(Val::Int(acc))
}

fn sym_name(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("")
}

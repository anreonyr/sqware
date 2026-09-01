//! lisp — 教学 Lisp 语言内核 + shell 适配（独占控制台输入）。
//!
//! 结构（第 2 关批准）：[`Val`] / [`Sym`] / [`Core`] / [`FnDef`]；原语（第 3/4 关
//! 批准）：[`Core::intern`] / [`Core::read`] / [`Core::eval`] / [`Core::print`] +
//! [`repl`] 适配。**无闭包**：函数值不捕获定义环境，调用时压参数帧进当前帧链
//! （`Core.frames`），`define` 恒写全局帧（`Core.globals`）——递归因此天然可用。
//!
//! 值共享：环境（globals/frames）持 [`Rc<Val>`]；求值全程**引用输入、只重建浅值**
//! （[`Core::eval_rc`]），函数体经 `Rc` 复用（`sp_lambda` 定义时克隆一次，调用链
//! 零深拷贝）——递归调用 O(深度) 栈帧，不再随表达式树放大。
//!
//! 子集：整数 / 符号 / `nil`（假 + 空表）/ `t`；`quote car cdr cons eq? + - * /(整除)
//! if define lambda`；`(exit)` 与 Ctrl+D 退出。错误域 [`LispError`] 五变体，
//! REPL 出错打印后继续。
//!
//! 控制字符（repl 行收集）：`\n`/`\r` 行结束、退格 `\x08`/`\x7f`、Ctrl+C 清行、
//! Ctrl+D EOF 退出。

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::env;

/// Lisp 错误（显式失败域；五变体见第 3 关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LispError {
    /// 词法/语法错：括号未闭合、非法 token、行内残留。
    Parse,
    /// 未定义符号（帧链与全局帧均无，且非内建）。
    Unbound,
    /// 参数个数不符（内建 / 用户函数 / 特殊形式）。
    Arity,
    /// 结构不当（特殊形式部件错位、car/cdr 空表、除零、非正规列表）。
    BadForm,
    /// 函数位置的值不是函数。
    NotCallable,
}

/// 符号（intern 表下标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sym(pub usize);

/// 函数值：参数表 + 体（无闭包：不捕获定义环境；体经 Rc 共享，调用零深拷贝）。
#[derive(Debug, Clone)]
pub struct FnDef {
    params: Vec<Sym>,
    body: Rc<Val>,
}

/// 值。
#[derive(Debug, Clone)]
pub enum Val {
    Int(i64),
    Sym(Sym),
    Nil,
    T,
    Cons(Box<(Val, Val)>),
    Fn(Box<FnDef>),
}

/// 语言内核状态（read/eval/print 共享可变状态）。
pub struct Core {
    /// 符号表：id → 名字（intern 登记；print 取名）。
    syms: Vec<Vec<u8>>,
    /// 全局帧（define 落点；查名在帧链之后；值 Rc 共享）。
    globals: Vec<(Sym, Rc<Val>)>,
    /// 调用帧链（无闭包：函数体在当前帧链 ⊕ 全局帧上求值）。
    frames: Vec<Vec<(Sym, Rc<Val>)>>,
}

/// 预登记名单（构造即 intern；id 即登记序，[`Core::bi`] 线性取回）。
const BUILTINS: &[&str] = &[
    "t", "nil", "quote", "car", "cdr", "cons", "eq?", "+", "-", "*", "/", "if", "define", "lambda",
    "exit",
];

impl Core {
    /// 构造：空符号表 + 预登记内建/常量名。
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

    /// 名称 → 符号 id（登记或取既有）。
    pub fn intern(&mut self, name: &[u8]) -> Sym {
        for (i, n) in self.syms.iter().enumerate() {
            if n.as_slice() == name {
                return Sym(i);
            }
        }
        self.syms.push(name.to_vec());
        Sym(self.syms.len() - 1)
    }

    /// 预登记名 → 符号 id（仅限 [`BUILTINS`]，命中恒成立）。
    fn bi(&self, name: &str) -> Sym {
        for (i, n) in self.syms.iter().enumerate() {
            if n.as_slice() == name.as_bytes() {
                return Sym(i);
            }
        }
        unreachable!("builtin not interned: {name}")
    }

    /// 一行字节 → 一个语法树；行内残留非空白 → Parse。
    pub fn read(&mut self, line: &[u8]) -> Result<Val, LispError> {
        let mut p = Parser {
            core: self,
            line,
            pos: 0,
        };
        let v = p.form()?;
        p.skip_ws();
        if p.pos != p.line.len() {
            return Err(LispError::Parse);
        }
        Ok(v)
    }

    /// 求值（拥有值入口）：委托引用版 [`Self::eval_rc`]。
    pub fn eval(&mut self, val: Val) -> Result<Val, LispError> {
        self.eval_rc(&val)
    }

    /// 核心求值（**引用输入**）：原子重建浅值；符号沿 Rc 展开（查名零拷贝）；
    /// 列表走应用。函数体/参数树全程不被深拷贝（唯一深拷贝在 `sp_lambda`
    /// 定义时一次与 `quote` 取引用树一次）。
    fn eval_rc(&mut self, v: &Val) -> Result<Val, LispError> {
        match v {
            Val::Int(n) => Ok(Val::Int(*n)),
            Val::T => Ok(Val::T),
            Val::Nil => Ok(Val::Nil),
            // Fn 值：浅拷贝（body/params 经 Rc 共享）
            Val::Fn(def) => Ok(Val::Fn(def.clone())),
            Val::Sym(s) => {
                // 内建名即函数值（不展开；lookup 对未绑定才报错）
                if self.is_builtin(*s) {
                    return Ok(Val::Sym(*s));
                }
                let r = self.lookup(*s)?;
                self.eval_rc(&r)
            }
            Val::Cons(_) => self.apply(v),
        }
    }

    /// 符号求值：帧链自顶向下 → 全局帧 → 常量（t/nil）→ 内建名返回自身
    /// （函数值）→ 未绑定。返回 **Rc 共享**（零深拷贝）。
    fn lookup(&self, s: Sym) -> Result<Rc<Val>, LispError> {
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

    /// 特殊形式名（quote/if/define/lambda）与内建名（car…/）总表——t/nil 除外。
    fn is_builtin(&self, s: Sym) -> bool {
        for n in BUILTINS {
            if *n != "t" && *n != "nil" && s == self.bi(n) {
                return true;
            }
        }
        false
    }

    /// 应用列表（引用输入）：头为特殊形式符号 → 特殊路径；否则求值头得函数、
    /// 求值实参（引用）后应用——实参树不深拷贝。
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
                _ => return Err(LispError::BadForm), // 非正规列表尾部
            }
        }
        self.apply_fn(f, args)
    }

    fn sp_quote(&mut self, rest: &Val) -> Result<Val, LispError> {
        match rest {
            Val::Cons(b) if matches!(b.1, Val::Nil) => Ok(b.0.clone()), // 引用树取用
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
        // 体克隆一次（定义时；此后 Rc 共享，调用零深拷贝）
        Ok(Val::Fn(Box::new(FnDef {
            params: ps,
            body: Rc::new(body.clone()),
        })))
    }

    /// 应用：用户函数压参数帧求值体后弹帧；内建按名分派；其余不可调用。
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
                                return Err(LispError::BadForm); // 除零
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

    /// 值 → 文本：整数 / 符号名 / `()` / `t` / `#<fn>` / 列表递归。
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
                            // 非正规列表尾部（教学不产生；留完整可打印性）
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

// ── 解析器（借用 Core 做 intern）──

struct Parser<'a> {
    core: &'a mut Core,
    line: &'a [u8],
    pos: usize,
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
        self.pos += 1; // '('
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(LispError::Parse), // 未闭合
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

/// 数字 token：全数字，或 `-` 后至少一个数字（单独 `-` 是符号=减法内建）。
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

// ── 求值辅助 ──────────────────────────────────────────

/// 引用列表取首元素与其尾部（实参/部件解构的公共拆法）。
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

/// 符号名视图（仅内部分派用；不分配）。
fn sym_name(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("")
}

// ── shell 适配（独占生命周期）────────────────────────

/// REPL：`>` → [`env::get`] 一行（控制字符处理）→ read/eval → print；出错打印
/// 后继续；`(exit)` 与 Ctrl+D 退出。独占控制台输入（唯一 get 消费者）。
pub fn repl(core: &mut Core) -> ! {
    loop {
        let _ = env::put("> ");
        let mut line = Vec::new();
        loop {
            let b = env::get();
            match b {
                0x0a | 0x0d => break, // 行结束（\n / \r）
                0x08 | 0x7f => {
                    line.pop(); // 退格
                }
                0x03 => line.clear(), // Ctrl+C 清行
                0x04 => env::exit(),  // Ctrl+D EOF → 退出
                b => line.push(b),
            }
        }
        if line.iter().all(|b| matches!(b, b' ' | b'\t')) {
            continue; // 空行
        }
        match core.read(&line) {
            Err(e) => {
                // TODO(dbg): 临时诊断——打印送达行的原始字节
                let _ = env::put(&format!("parse error({e:?}): {line:02x?}\n"));
            }
            Ok(v) => {
                if is_command(core, &v, "exit") {
                    env::exit();
                }
                let defined = is_command(core, &v, "define");
                match core.eval(v) {
                    Err(e) => err_line(e),
                    Ok(r) if !defined => {
                        let _ = env::put(&core.print(&r));
                        let _ = env::put("\n");
                    }
                    Ok(_) => {} // define 求值成功不打印
                }
            }
        }
    }
}

/// 列表头的符号是否等于预登记名（shell 级命令判定）。
fn is_command(core: &Core, v: &Val, name: &str) -> bool {
    match v {
        Val::Cons(b) => matches!(&b.0, Val::Sym(s) if *s == core.bi(name)),
        _ => false,
    }
}

fn err_line(e: LispError) {
    let msg = match e {
        LispError::Parse => "parse error",
        LispError::Unbound => "unbound symbol",
        LispError::Arity => "wrong arity",
        LispError::BadForm => "bad form",
        LispError::NotCallable => "not callable",
    };
    let _ = env::put(msg);
    let _ = env::put("\n");
}

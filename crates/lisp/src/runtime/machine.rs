use super::{
    native::CallId,
    value::{Handle, Value},
};
use crate::{
    Span,
    syntax::lower::{Binding, Expr, ExprId, Module},
};
use alloc::{rc::Rc, string::String, vec::Vec};
#[derive(Clone)]
pub(crate) struct Context {
    pub module: Rc<Module>,
    pub env: Option<Handle>,
    pub span: Span,
}
pub(crate) enum QuoteWork {
    Visit(crate::syntax::reader::NodeId),
    List(usize, bool),
    Pair(Value),
    Quote,
}
pub(crate) enum State {
    Idle,
    Eval(ExprId, Context),
    Return(Value, Context),
    Quote {
        work: Vec<QuoteWork>,
        values: Vec<Value>,
        context: Context,
    },
    Builtin {
        work: crate::library::Task,
        context: Context,
    },
    Waiting {
        id: CallId,
        operation: usize,
        args: Vec<Value>,
        context: Context,
        issued: bool,
    },
}
pub(crate) enum Frame {
    If {
        yes: ExprId,
        no: ExprId,
        context: Context,
    },
    Sequence {
        rest: Vec<ExprId>,
        next: usize,
        context: Context,
    },
    Define {
        name: String,
        context: Context,
    },
    Assign {
        binding: Binding,
        context: Context,
    },
    Operator {
        args: Vec<ExprId>,
        context: Context,
    },
    Arguments {
        function: Value,
        args: Vec<ExprId>,
        next: usize,
        values: Vec<Value>,
        context: Context,
    },
}
impl Frame {
    pub fn context(&self) -> &Context {
        match self {
            Self::If { context, .. }
            | Self::Sequence { context, .. }
            | Self::Define { context, .. }
            | Self::Assign { context, .. }
            | Self::Operator { context, .. }
            | Self::Arguments { context, .. } => context,
        }
    }
    pub fn roots(&self, out: &mut Vec<Value>) {
        out.extend(self.context().env.map(Value::Object));
        if let Self::Arguments {
            function, values, ..
        } = self
        {
            out.push(*function);
            out.extend(values);
        }
    }
}
impl State {
    pub fn roots(&self, out: &mut Vec<Value>) {
        match self {
            Self::Eval(_, context) => out.extend(context.env.map(Value::Object)),
            Self::Return(value, context) => {
                out.push(*value);
                out.extend(context.env.map(Value::Object));
            }
            Self::Waiting { args, context, .. } => {
                out.extend(args);
                out.extend(context.env.map(Value::Object));
            }
            Self::Quote {
                work,
                values,
                context,
            } => {
                out.extend(values);
                out.extend(context.env.map(Value::Object));
                for task in work {
                    if let QuoteWork::Pair(value) = task {
                        out.push(*value);
                    }
                }
            }
            Self::Builtin { work, context } => {
                work.roots(out);
                out.extend(context.env.map(Value::Object));
            }
            Self::Idle => {}
        }
    }
}

use super::{engine::Engine, env::Environment, heap::Object, native::Call, value::RootValue};
use crate::{
    Diagnostic, ErrorKind, Step, library,
    syntax::reader::{Atom, Datum},
};
use alloc::{format, vec};
impl Engine {
    fn cell(&self, binding: &Binding, mut env: Option<Handle>) -> Result<Handle, ErrorKind> {
        match binding {
            Binding::Global(name) => self.globals.get(name).copied().ok_or(ErrorKind::Unbound),
            Binding::Local { depth, slot } => {
                for _ in 0..*depth {
                    env = match self.heap.get(env.ok_or(ErrorKind::Unbound)?) {
                        Some(Object::Environment(e)) => e.parent,
                        _ => return Err(ErrorKind::StaleValue),
                    };
                }
                match self.heap.get(env.ok_or(ErrorKind::Unbound)?) {
                    Some(Object::Environment(e)) => {
                        e.cells.get(*slot).copied().ok_or(ErrorKind::Unbound)
                    }
                    _ => Err(ErrorKind::StaleValue),
                }
            }
        }
    }
    fn frame(&mut self, frame: Frame) -> Result<(), ErrorKind> {
        if self.frames.len() >= self.limits.frames {
            return Err(ErrorKind::ResourceLimit);
        }
        self.frames.push(frame);
        self.peak_frames = self.peak_frames.max(self.frames.len());
        Ok(())
    }
    fn eval(&mut self, id: usize, mut context: Context) {
        context.span = context.module.expressions[id].span;
        self.state = State::Eval(id, context);
    }
    fn literal(&mut self, atom: &Atom) -> Result<Value, ErrorKind> {
        match atom {
            Atom::Integer(n) => Ok(Value::Integer(*n)),
            Atom::Boolean(b) => Ok(Value::Boolean(*b)),
            Atom::String(s) => self.alloc(Object::String(s.clone())),
            Atom::Symbol(s) => self.alloc(Object::Symbol(s.clone())),
            Atom::Bytes(b) => self.alloc(Object::Bytes(b.clone())),
        }
    }
    fn apply(
        &mut self,
        function: Value,
        args: Vec<Value>,
        context: Context,
    ) -> Result<(), ErrorKind> {
        match function {
            Value::Builtin(id) => {
                if let Some(work) = library::Task::new(id, &args)? {
                    self.state = State::Builtin { work, context };
                } else {
                    let value = library::apply(id, &args, &mut self.heap)?;
                    self.state = State::Return(value, context);
                }
            }
            Value::Native(id) => {
                let native = self.natives.get(id).ok_or(ErrorKind::StaleValue)?;
                if !native.arity.accepts(args.len()) {
                    return Err(ErrorKind::Arity);
                }
                let operation = native.operation;
                let id = CallId(self.next_call);
                self.next_call = self
                    .next_call
                    .checked_add(1)
                    .ok_or(ErrorKind::ResourceLimit)?;
                self.state = State::Waiting {
                    id,
                    operation,
                    args,
                    context,
                    issued: false,
                };
            }
            Value::Object(handle) => {
                let Some(Object::Closure {
                    module,
                    body,
                    params,
                    env,
                }) = self.heap.get(handle).cloned()
                else {
                    return Err(ErrorKind::Type);
                };
                if params != args.len() {
                    return Err(ErrorKind::Arity);
                }
                let mut cells = Vec::new();
                for value in args {
                    let Value::Object(cell) = self.alloc(Object::Cell(value))? else {
                        unreachable!()
                    };
                    cells.push(cell);
                }
                let Value::Object(env) =
                    self.alloc(Object::Environment(Environment { parent: env, cells }))?
                else {
                    unreachable!()
                };
                self.eval(
                    body,
                    Context {
                        module,
                        env: Some(env),
                        span: context.span,
                    },
                );
            }
            _ => return Err(ErrorKind::Type),
        }
        Ok(())
    }
    fn transition(&mut self) -> Result<Option<RootValue>, ErrorKind> {
        let state = core::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Eval(id, context) => {
                let expr = context.module.expressions[id].expr.clone();
                match expr {
                    Expr::Literal(atom) => {
                        let value = self.literal(&atom)?;
                        self.state = State::Return(value, context);
                    }
                    Expr::Quote(node) => {
                        self.state = State::Quote {
                            work: vec![QuoteWork::Visit(node)],
                            values: Vec::new(),
                            context,
                        };
                    }
                    Expr::Reference(binding) => {
                        let cell = self.cell(&binding, context.env)?;
                        let Some(Object::Cell(value)) = self.heap.get(cell) else {
                            return Err(ErrorKind::StaleValue);
                        };
                        self.state = State::Return(*value, context);
                    }
                    Expr::If { test, yes, no } => {
                        self.frame(Frame::If {
                            yes,
                            no,
                            context: context.clone(),
                        })?;
                        self.eval(test, context);
                    }
                    Expr::Sequence(parts) => {
                        if let Some(&first) = parts.first() {
                            if parts.len() > 1 {
                                self.frame(Frame::Sequence {
                                    rest: parts,
                                    next: 1,
                                    context: context.clone(),
                                })?;
                            }
                            self.eval(first, context);
                        } else {
                            self.state = State::Return(Value::Nil, context);
                        }
                    }
                    Expr::Lambda { params, body } => {
                        let value = self.alloc(Object::Closure {
                            module: context.module.clone(),
                            params,
                            body,
                            env: context.env,
                        })?;
                        self.state = State::Return(value, context);
                    }
                    Expr::Call { function, args } => {
                        self.frame(Frame::Operator {
                            args,
                            context: context.clone(),
                        })?;
                        self.eval(function, context);
                    }
                    Expr::Define { name, value } => {
                        self.frame(Frame::Define {
                            name,
                            context: context.clone(),
                        })?;
                        self.eval(value, context);
                    }
                    Expr::Assign { binding, value } => {
                        self.frame(Frame::Assign {
                            binding,
                            context: context.clone(),
                        })?;
                        self.eval(value, context);
                    }
                }
            }
            State::Builtin { mut work, context } => {
                self.state = match work.step(&mut self.heap)? {
                    Some(value) => State::Return(value, context),
                    None => State::Builtin { work, context },
                };
            }
            State::Quote {
                mut work,
                mut values,
                context,
            } => {
                match work.pop() {
                    Some(QuoteWork::Visit(id)) => match &context.module.form.nodes[id].datum {
                        Datum::Atom(a) => values.push(self.literal(a)?),
                        Datum::Quote(id) => {
                            work.push(QuoteWork::Quote);
                            work.push(QuoteWork::Visit(*id));
                        }
                        Datum::List { items, tail } => {
                            work.push(QuoteWork::List(items.len(), tail.is_some()));
                            if let Some(tail) = tail {
                                work.push(QuoteWork::Visit(*tail));
                            }
                            for &id in items.iter().rev() {
                                work.push(QuoteWork::Visit(id));
                            }
                        }
                    },
                    Some(QuoteWork::List(n, dotted)) => {
                        let tail = if dotted {
                            values.pop().ok_or(ErrorKind::Syntax)?
                        } else {
                            Value::Nil
                        };
                        let items = values.split_off(values.len() - n);
                        for value in items {
                            work.push(QuoteWork::Pair(value));
                        }
                        values.push(tail);
                    }
                    Some(QuoteWork::Pair(first)) => {
                        let rest = values.pop().ok_or(ErrorKind::Syntax)?;
                        values.push(self.alloc(Object::Pair(first, rest))?);
                    }
                    Some(QuoteWork::Quote) => {
                        let value = values.pop().ok_or(ErrorKind::Syntax)?;
                        let tail = self.alloc(Object::Pair(value, Value::Nil))?;
                        let symbol = self.alloc(Object::Symbol(String::from("quote")))?;
                        values.push(self.alloc(Object::Pair(symbol, tail))?);
                    }
                    None => {
                        self.state = State::Return(values.pop().ok_or(ErrorKind::Syntax)?, context);
                        return Ok(None);
                    }
                }
                self.state = State::Quote {
                    work,
                    values,
                    context,
                };
            }
            State::Return(value, _context) => match self.frames.pop() {
                None => return Ok(Some(self.pin(value))),
                Some(Frame::If { yes, no, context }) => self.eval(
                    if value == Value::Boolean(false) {
                        no
                    } else {
                        yes
                    },
                    context,
                ),
                Some(Frame::Sequence {
                    rest,
                    next,
                    context,
                }) => {
                    let expr = rest[next];
                    if next + 1 < rest.len() {
                        self.frame(Frame::Sequence {
                            rest,
                            next: next + 1,
                            context: context.clone(),
                        })?;
                    }
                    self.eval(expr, context);
                }
                Some(Frame::Define { name, context }) => {
                    self.define(&name, value)?;
                    self.state = State::Return(value, context);
                }
                Some(Frame::Assign { binding, context }) => {
                    let cell = self.cell(&binding, context.env)?;
                    let Some(Object::Cell(target)) = self.heap.get_mut(cell) else {
                        return Err(ErrorKind::StaleValue);
                    };
                    *target = value;
                    self.state = State::Return(value, context);
                }
                Some(Frame::Operator { args, context }) => {
                    if args.is_empty() {
                        self.apply(value, Vec::new(), context)?;
                    } else {
                        let first = args[0];
                        self.frame(Frame::Arguments {
                            function: value,
                            args,
                            next: 1,
                            values: Vec::new(),
                            context: context.clone(),
                        })?;
                        self.eval(first, context);
                    }
                }
                Some(Frame::Arguments {
                    function,
                    args,
                    next,
                    mut values,
                    context,
                }) => {
                    values.push(value);
                    if next == args.len() {
                        self.apply(function, values, context)?;
                    } else {
                        let expr = args[next];
                        self.frame(Frame::Arguments {
                            function,
                            args,
                            next: next + 1,
                            values,
                            context: context.clone(),
                        })?;
                        self.eval(expr, context);
                    }
                }
            },
            other => self.state = other,
        }
        Ok(None)
    }
    pub fn step(&mut self, budget: usize) -> Step {
        for _ in 0..budget {
            if matches!(self.state, State::Idle) {
                return Step::Yielded;
            }
            if let State::Waiting {
                id,
                operation,
                args,
                issued,
                ..
            } = &mut self.state
            {
                if *issued {
                    return Step::Yielded;
                }
                *issued = true;
                let id = *id;
                let operation = *operation;
                let values = args.clone();
                return Step::Request(Call {
                    id,
                    operation,
                    arguments: values.into_iter().map(|v| self.pin(v)).collect(),
                });
            }
            if self.heap.used > self.heap.object_budget() / 2 {
                self.collect();
            }
            let context = match &self.state {
                State::Eval(_, c)
                | State::Return(_, c)
                | State::Quote { context: c, .. }
                | State::Builtin { context: c, .. } => c.clone(),
                _ => unreachable!(),
            };
            match self.transition() {
                Ok(Some(value)) => return Step::Done(value),
                Ok(None) => {}
                Err(kind) => {
                    let mut error = Diagnostic::new(
                        kind,
                        format!("{kind:?} during evaluation"),
                        &context.module.form.source,
                        context.span,
                    );
                    error.trace.extend(self.frames.iter().rev().map(|frame| {
                        crate::diagnostic::TraceFrame {
                            source: frame.context().module.form.source.clone(),
                            span: frame.context().span,
                        }
                    }));
                    self.cancel();
                    return Step::Failed(error);
                }
            }
        }
        Step::Yielded
    }
}

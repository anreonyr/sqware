use super::{
    heap::{Heap, Object},
    machine::{Context, Frame, State},
    native::{Arity, Call, CallId, ForeignId, Native, NativeResult},
    value::{Handle, RootValue, Roots, Value, ValueKind},
};
use crate::{
    Diagnostic, ErrorKind, Source, Span, library,
    syntax::{lower, reader::Form},
};
use alloc::{collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use core::cell::RefCell;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub input: usize,
    pub depth: usize,
    pub frames: usize,
    pub heap: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            input: 64 * 1024,
            depth: 1024,
            frames: 4096,
            heap: 4 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub enum Step {
    Yielded,
    Request(Call),
    Done(RootValue),
    Failed(Diagnostic),
}
pub struct Engine {
    pub(super) limits: Limits,
    pub(super) heap: Heap,
    pub(super) roots: Rc<RefCell<Roots>>,
    pub(super) globals: BTreeMap<String, Handle>,
    pub(super) natives: Vec<Native>,
    pub(super) state: State,
    pub(super) frames: Vec<Frame>,
    pub(super) next_call: u64,
    pub(super) released: Vec<ForeignId>,
    pub(super) peak_frames: usize,
}
impl Engine {
    pub fn new(limits: Limits) -> Result<Self, Diagnostic> {
        let mut engine = Self {
            limits,
            heap: Heap::new(limits.heap),
            roots: Rc::new(RefCell::new(Roots::default())),
            globals: BTreeMap::new(),
            natives: Vec::new(),
            state: State::Idle,
            frames: Vec::new(),
            next_call: 0,
            released: Vec::new(),
            peak_frames: 0,
        };
        for (id, builtin) in library::BUILTINS.iter().enumerate() {
            engine
                .define(builtin.name, Value::Builtin(id))
                .map_err(|kind| engine.api_error(kind, "cannot install language library"))?;
        }
        Ok(engine)
    }
    pub(super) fn api_error(&self, kind: ErrorKind, message: &str) -> Diagnostic {
        Diagnostic::new(kind, message, &Source::new("<host>", ""), Span::default())
    }
    pub(super) fn pin(&self, value: Value) -> RootValue {
        RootValue::new(value, &self.roots)
    }
    pub(super) fn value(&self, root: &RootValue) -> Result<Value, Diagnostic> {
        if !Rc::ptr_eq(&root.roots, &self.roots) {
            return Err(self.api_error(ErrorKind::StaleValue, "value belongs to another engine"));
        }
        Ok(root.value)
    }
    pub fn integer(&self, n: i64) -> RootValue {
        self.pin(Value::Integer(n))
    }
    pub fn boolean(&self, b: bool) -> RootValue {
        self.pin(Value::Boolean(b))
    }
    pub fn nil(&self) -> RootValue {
        self.pin(Value::Nil)
    }
    fn object(&mut self, object: Object) -> Result<RootValue, Diagnostic> {
        self.collect();
        let value = self
            .heap
            .alloc(object)
            .map(Value::Object)
            .map_err(|_| self.api_error(ErrorKind::ResourceLimit, "heap limit exceeded"))?;
        Ok(self.pin(value))
    }
    pub fn string(&mut self, value: &str) -> Result<RootValue, Diagnostic> {
        self.object(Object::String(String::from(value)))
    }
    pub fn bytes(&mut self, value: &[u8]) -> Result<RootValue, Diagnostic> {
        self.object(Object::Bytes(value.to_vec()))
    }
    pub fn symbol(&mut self, value: &str) -> Result<RootValue, Diagnostic> {
        self.object(Object::Symbol(String::from(value)))
    }
    pub fn cons(&mut self, first: &RootValue, rest: &RootValue) -> Result<RootValue, Diagnostic> {
        let first = self.value(first)?;
        let rest = self.value(rest)?;
        self.object(Object::Pair(first, rest))
    }
    pub fn list(&mut self, values: &[RootValue]) -> Result<RootValue, Diagnostic> {
        let mut out = self.nil();
        for value in values.iter().rev() {
            out = self.cons(value, &out)?;
        }
        Ok(out)
    }
    pub fn foreign(&mut self, id: ForeignId, kind: &str) -> Result<RootValue, Diagnostic> {
        self.object(Object::Foreign {
            id,
            kind: String::from(kind),
        })
    }
    pub fn foreign_id(&self, value: &RootValue) -> Result<Option<ForeignId>, Diagnostic> {
        Ok(match self.value(value)? {
            Value::Object(h) => match self.heap.get(h) {
                Some(Object::Foreign { id, .. }) => Some(*id),
                _ => None,
            },
            _ => None,
        })
    }
    pub fn text(&self, value: &RootValue) -> Result<Option<&str>, Diagnostic> {
        Ok(match self.value(value)? {
            Value::Object(h) => match self.heap.get(h) {
                Some(Object::String(s) | Object::Symbol(s)) => Some(s),
                _ => None,
            },
            _ => None,
        })
    }
    pub fn byte_slice(&self, value: &RootValue) -> Result<Option<&[u8]>, Diagnostic> {
        Ok(match self.value(value)? {
            Value::Object(h) => match self.heap.get(h) {
                Some(Object::Bytes(b)) => Some(b),
                _ => None,
            },
            _ => None,
        })
    }
    pub fn pair(&self, value: &RootValue) -> Result<Option<(RootValue, RootValue)>, Diagnostic> {
        Ok(match self.value(value)? {
            Value::Object(h) => match self.heap.get(h) {
                Some(Object::Pair(a, b)) => Some((self.pin(*a), self.pin(*b))),
                _ => None,
            },
            _ => None,
        })
    }
    pub fn kind(&self, root: &RootValue) -> Result<ValueKind, Diagnostic> {
        Ok(match self.value(root)? {
            Value::Nil => ValueKind::Nil,
            Value::Integer(_) => ValueKind::Integer,
            Value::Boolean(_) => ValueKind::Boolean,
            Value::Builtin(_) | Value::Native(_) => ValueKind::Function,
            Value::Object(h) => match self
                .heap
                .get(h)
                .ok_or_else(|| self.api_error(ErrorKind::StaleValue, "stale value"))?
            {
                Object::Symbol(_) => ValueKind::Symbol,
                Object::String(_) => ValueKind::String,
                Object::Bytes(_) => ValueKind::Bytes,
                Object::Pair(..) => ValueKind::Pair,
                Object::Closure { .. } => ValueKind::Function,
                Object::Foreign { .. } => ValueKind::Foreign,
                _ => return Err(self.api_error(ErrorKind::Type, "internal object")),
            },
        })
    }
    pub fn register(
        &mut self,
        name: &str,
        operation: usize,
        arity: Arity,
    ) -> Result<(), Diagnostic> {
        if arity.max.is_some_and(|max| max < arity.min) {
            return Err(self.api_error(ErrorKind::Arity, "invalid arity"));
        }
        let id = self.natives.len();
        self.natives.push(Native { operation, arity });
        self.define(name, Value::Native(id))
            .map_err(|kind| self.api_error(kind, "cannot register host function"))
    }
    pub fn start(&mut self, form: Form) -> Result<(), Diagnostic> {
        if !matches!(self.state, State::Idle) {
            return Err(self.api_error(ErrorKind::Busy, "evaluation already active"));
        }
        if form.source.text().len() > self.limits.input {
            return Err(Diagnostic::new(
                ErrorKind::ResourceLimit,
                "input limit exceeded",
                &form.source,
                form.span(),
            ));
        }
        if form.depth > self.limits.depth {
            return Err(Diagnostic::new(
                ErrorKind::ResourceLimit,
                "structure nesting limit exceeded",
                &form.source,
                form.span(),
            ));
        }
        let module = lower::lower(form)?;
        let span = module.expressions[module.root].span;
        self.state = State::Eval(
            module.root,
            Context {
                module,
                env: None,
                span,
            },
        );
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.state = State::Idle;
        self.frames.clear();
    }
    pub fn resume(&mut self, id: CallId, result: NativeResult) -> Result<(), Diagnostic> {
        let context = match &self.state {
            State::Waiting {
                id: pending,
                context,
                ..
            } if *pending == id => context.clone(),
            _ => return Err(self.api_error(ErrorKind::StaleCall, "call is no longer pending")),
        };
        match result {
            Ok(value) => {
                let value = self.value(&value)?;
                self.state = State::Return(value, context);
            }
            Err(error) => {
                let mut diagnostic = Diagnostic::new(
                    ErrorKind::Host,
                    error.0,
                    &context.module.form.source,
                    context.span,
                );
                diagnostic
                    .trace
                    .extend(
                        self.frames
                            .iter()
                            .rev()
                            .map(|frame| crate::diagnostic::TraceFrame {
                                source: frame.context().module.form.source.clone(),
                                span: frame.context().span,
                            }),
                    );
                self.cancel();
                return Err(diagnostic);
            }
        }
        Ok(())
    }
    pub fn collect(&mut self) {
        let mut roots: Vec<Value> = self.roots.borrow().values.values().copied().collect();
        roots.extend(self.globals.values().copied().map(Value::Object));
        self.state.roots(&mut roots);
        for frame in &self.frames {
            frame.roots(&mut roots);
        }
        self.released.extend(self.heap.collect(roots));
    }
    pub fn take_released(&mut self) -> Vec<ForeignId> {
        core::mem::take(&mut self.released)
    }
    pub fn heap_used(&self) -> usize {
        self.heap.used
    }
    pub fn peak_frames(&self) -> usize {
        self.peak_frames
    }
    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }
    pub(super) fn alloc(&mut self, object: Object) -> Result<Value, ErrorKind> {
        self.heap
            .alloc(object)
            .map(Value::Object)
            .map_err(|_| ErrorKind::ResourceLimit)
    }
    pub(super) fn define(&mut self, name: &str, value: Value) -> Result<(), ErrorKind> {
        if let Some(&cell) = self.globals.get(name) {
            match self.heap.get_mut(cell) {
                Some(Object::Cell(v)) => *v = value,
                _ => return Err(ErrorKind::StaleValue),
            }
        } else {
            let charge = name.len().checked_add(64).ok_or(ErrorKind::ResourceLimit)?;
            self.heap
                .charge(charge)
                .map_err(|_| ErrorKind::ResourceLimit)?;
            let cell = match self.heap.alloc(Object::Cell(value)) {
                Ok(cell) => cell,
                Err(_) => {
                    self.heap.used -= charge;
                    return Err(ErrorKind::ResourceLimit);
                }
            };
            self.globals.insert(String::from(name), cell);
        }
        Ok(())
    }
}

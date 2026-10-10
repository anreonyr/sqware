use super::{
    engine::Engine,
    heap::Object,
    value::{RootValue, Value},
};
use crate::{Diagnostic, ErrorKind};
use alloc::{format, string::String, vec};
impl Engine {
    pub fn display(&self, root: &RootValue) -> Result<String, Diagnostic> {
        enum Work {
            Value(Value),
            Text(&'static str),
            Tail(Value),
        }
        let mut work = vec![Work::Value(self.value(root)?)];
        let mut out = String::new();
        while let Some(task) = work.pop() {
            if out.len() > self.limits.heap {
                return Err(self.api_error(ErrorKind::ResourceLimit, "display limit exceeded"));
            }
            match task {
                Work::Text(text) => out.push_str(text),
                Work::Tail(Value::Nil) => out.push(')'),
                Work::Tail(value) => match value {
                    Value::Object(h) if matches!(self.heap.get(h), Some(Object::Pair(..))) => {
                        let Some(Object::Pair(a, b)) = self.heap.get(h) else {
                            unreachable!()
                        };
                        out.push(' ');
                        work.push(Work::Tail(*b));
                        work.push(Work::Value(*a));
                    }
                    _ => {
                        out.push_str(" . ");
                        work.push(Work::Text(")"));
                        work.push(Work::Value(value));
                    }
                },
                Work::Value(value) => match value {
                    Value::Nil => out.push_str("()"),
                    Value::Integer(n) => out.push_str(&format!("{n}")),
                    Value::Boolean(b) => out.push_str(if b { "#t" } else { "#f" }),
                    Value::Builtin(_) | Value::Native(_) => out.push_str("<function>"),
                    Value::Object(h) => match self
                        .heap
                        .get(h)
                        .ok_or_else(|| self.api_error(ErrorKind::StaleValue, "stale value"))?
                    {
                        Object::Symbol(s) => out.push_str(s),
                        Object::String(s) => {
                            out.push('"');
                            for ch in s.chars() {
                                match ch {
                                    '"' => out.push_str("\\\""),
                                    '\\' => out.push_str("\\\\"),
                                    '\n' => out.push_str("\\n"),
                                    '\r' => out.push_str("\\r"),
                                    '\t' => out.push_str("\\t"),
                                    _ => out.push(ch),
                                }
                            }
                            out.push('"');
                        }
                        Object::Bytes(b) => {
                            out.push_str("#u8(");
                            for (i, byte) in b.iter().enumerate() {
                                if i != 0 {
                                    out.push(' ');
                                }
                                out.push_str(&format!("{byte}"));
                            }
                            out.push(')');
                        }
                        Object::Pair(a, b) => {
                            out.push('(');
                            work.push(Work::Tail(*b));
                            work.push(Work::Value(*a));
                        }
                        Object::Closure { .. } => out.push_str("<function>"),
                        Object::Foreign { kind, .. } => {
                            out.push('<');
                            out.push_str(kind);
                            out.push('>');
                        }
                        _ => return Err(self.api_error(ErrorKind::Type, "internal object")),
                    },
                },
            }
        }
        Ok(out)
    }
}

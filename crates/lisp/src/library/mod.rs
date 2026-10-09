use crate::{
    ErrorKind,
    runtime::{
        heap::{Heap, Object},
        native::Arity,
        value::Value,
    },
};
use alloc::{string::String, vec::Vec};
#[derive(Clone, Copy)]
pub(crate) struct Builtin {
    pub name: &'static str,
    pub arity: Arity,
}
pub(crate) const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "+",
        arity: Arity::variadic(0),
    },
    Builtin {
        name: "-",
        arity: Arity::variadic(1),
    },
    Builtin {
        name: "*",
        arity: Arity::variadic(0),
    },
    Builtin {
        name: "/",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: "=",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: "<",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: ">",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: "<=",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: ">=",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: "not",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "cons",
        arity: Arity::fixed(2),
    },
    Builtin {
        name: "car",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "cdr",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "null?",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "pair?",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "list",
        arity: Arity::variadic(0),
    },
    Builtin {
        name: "number?",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "boolean?",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "string-length",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "bytes-length",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "string-append",
        arity: Arity::variadic(0),
    },
    Builtin {
        name: "bytes-append",
        arity: Arity::variadic(0),
    },
    Builtin {
        name: "string->bytes",
        arity: Arity::fixed(1),
    },
    Builtin {
        name: "bytes->string",
        arity: Arity::fixed(1),
    },
];
fn integer(value: Value) -> Result<i64, ErrorKind> {
    if let Value::Integer(n) = value {
        Ok(n)
    } else {
        Err(ErrorKind::Type)
    }
}
fn object(heap: &Heap, value: Value) -> Result<&Object, ErrorKind> {
    if let Value::Object(handle) = value {
        heap.get(handle).ok_or(ErrorKind::StaleValue)
    } else {
        Err(ErrorKind::Type)
    }
}
fn alloc(heap: &mut Heap, object: Object) -> Result<Value, ErrorKind> {
    heap.alloc(object)
        .map(Value::Object)
        .map_err(|_| ErrorKind::ResourceLimit)
}
pub(crate) fn apply(id: usize, args: &[Value], heap: &mut Heap) -> Result<Value, ErrorKind> {
    let op = BUILTINS[id];
    if !op.arity.accepts(args.len()) {
        return Err(ErrorKind::Arity);
    }
    match op.name {
        "-" => integer(args[0])?
            .checked_neg()
            .map(Value::Integer)
            .ok_or(ErrorKind::Arithmetic),
        "/" => integer(args[0])?
            .checked_div(integer(args[1])?)
            .map(Value::Integer)
            .ok_or(ErrorKind::Arithmetic),
        "=" | "<" | ">" | "<=" | ">=" => {
            let a = integer(args[0])?;
            let b = integer(args[1])?;
            Ok(Value::Boolean(match op.name {
                "=" => a == b,
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                _ => a >= b,
            }))
        }
        "not" => Ok(Value::Boolean(args[0] == Value::Boolean(false))),
        "cons" => alloc(heap, Object::Pair(args[0], args[1])),
        "car" | "cdr" => match object(heap, args[0])? {
            Object::Pair(a, b) => Ok(if op.name == "car" { *a } else { *b }),
            _ => Err(ErrorKind::Type),
        },
        "null?" => Ok(Value::Boolean(args[0] == Value::Nil)),
        "pair?" => Ok(Value::Boolean(matches!(
            object(heap, args[0]),
            Ok(Object::Pair(..))
        ))),
        "number?" => Ok(Value::Boolean(matches!(args[0], Value::Integer(_)))),
        "boolean?" => Ok(Value::Boolean(matches!(args[0], Value::Boolean(_)))),
        "bytes-length" => match object(heap, args[0])? {
            Object::Bytes(b) => Ok(Value::Integer(b.len() as i64)),
            _ => Err(ErrorKind::Type),
        },
        _ => Err(ErrorKind::Unbound),
    }
}

mod task;
pub(crate) use task::Task;

use super::*;
/// Work retained by the machine, so large allocations remain interruptible.
pub(crate) enum Task {
    List {
        args: Vec<Value>,
        tail: Value,
    },
    Copy {
        args: Vec<Value>,
        index: usize,
        offset: usize,
        output: Vec<u8>,
        text: bool,
        input_text: bool,
    },
    Length {
        value: Value,
        offset: usize,
        chars: i64,
    },
    Arithmetic {
        args: Vec<Value>,
        index: usize,
        result: i64,
        op: &'static str,
    },
}
impl Task {
    pub fn new(id: usize, args: &[Value]) -> Result<Option<Self>, ErrorKind> {
        let builtin = BUILTINS.get(id).ok_or(ErrorKind::Unbound)?;
        if !builtin.arity.accepts(args.len()) {
            return Err(ErrorKind::Arity);
        }
        Ok(match builtin.name {
            "list" => Some(Self::List {
                args: args.to_vec(),
                tail: Value::Nil,
            }),
            "string-append" | "bytes-append" | "string->bytes" | "bytes->string" => {
                Some(Self::Copy {
                    args: args.to_vec(),
                    index: 0,
                    offset: 0,
                    output: Vec::new(),
                    text: matches!(builtin.name, "string-append" | "bytes->string"),
                    input_text: matches!(builtin.name, "string-append" | "string->bytes"),
                })
            }
            "string-length" => Some(Self::Length {
                value: args[0],
                offset: 0,
                chars: 0,
            }),
            "+" | "-" | "*" if !(builtin.name == "-" && args.len() == 1) => {
                Some(Self::Arithmetic {
                    args: args.to_vec(),
                    index: usize::from(builtin.name == "-"),
                    result: match builtin.name {
                        "*" => 1,
                        "-" => integer(args[0])?,
                        _ => 0,
                    },
                    op: builtin.name,
                })
            }
            _ => None,
        })
    }
    pub fn roots(&self, out: &mut Vec<Value>) {
        match self {
            Self::List { args, tail } => {
                out.extend(args);
                out.push(*tail);
            }
            Self::Copy { args, .. } | Self::Arithmetic { args, .. } => out.extend(args),
            Self::Length { value, .. } => out.push(*value),
        }
    }
    pub fn step(&mut self, heap: &mut Heap) -> Result<Option<Value>, ErrorKind> {
        match self {
            Self::List { args, tail } => {
                if let Some(first) = args.pop() {
                    *tail = alloc(heap, Object::Pair(first, *tail))?;
                    Ok(None)
                } else {
                    Ok(Some(*tail))
                }
            }
            Self::Copy {
                args,
                index,
                offset,
                output,
                text,
                input_text,
            } => {
                if *index == args.len() {
                    let bytes = core::mem::take(output);
                    return alloc(
                        heap,
                        if *text {
                            Object::String(String::from_utf8(bytes).map_err(|_| ErrorKind::Type)?)
                        } else {
                            Object::Bytes(bytes)
                        },
                    )
                    .map(Some);
                }
                let bytes = match (object(heap, args[*index])?, *input_text) {
                    (Object::String(s), true) => s.as_bytes(),
                    (Object::Bytes(b), false) => b.as_slice(),
                    _ => return Err(ErrorKind::Type),
                };
                let end = (*offset + 256).min(bytes.len());
                if end - *offset
                    > heap
                        .limit
                        .saturating_sub(heap.used)
                        .saturating_sub(output.len())
                {
                    return Err(ErrorKind::ResourceLimit);
                }
                output
                    .try_reserve(end - *offset)
                    .map_err(|_| ErrorKind::ResourceLimit)?;
                output.extend_from_slice(&bytes[*offset..end]);
                *offset = end;
                if end == bytes.len() {
                    *index += 1;
                    *offset = 0;
                }
                Ok(None)
            }
            Self::Length {
                value,
                offset,
                chars,
            } => {
                let Object::String(text) = object(heap, *value)? else {
                    return Err(ErrorKind::Type);
                };
                let mut end = (*offset + 256).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                *chars += text[*offset..end].chars().count() as i64;
                *offset = end;
                Ok((end == text.len()).then_some(Value::Integer(*chars)))
            }
            Self::Arithmetic {
                args,
                index,
                result,
                op,
            } => {
                if *index == args.len() {
                    return Ok(Some(Value::Integer(*result)));
                }
                let value = integer(args[*index])?;
                *result = match *op {
                    "+" => result.checked_add(value),
                    "-" => result.checked_sub(value),
                    _ => result.checked_mul(value),
                }
                .ok_or(ErrorKind::Arithmetic)?;
                *index += 1;
                Ok(None)
            }
        }
    }
}

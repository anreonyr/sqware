use super::lexer::{self, Token};
use crate::{Diagnostic, ErrorKind, Source, Span};
use alloc::{rc::Rc, string::String, vec::Vec};

pub(crate) type NodeId = usize;
#[derive(Clone, Debug)]
pub(crate) enum Atom {
    Integer(i64),
    Boolean(bool),
    String(String),
    Symbol(String),
    Bytes(Vec<u8>),
}
#[derive(Clone, Debug)]
pub(crate) enum Datum {
    Atom(Atom),
    List {
        items: Vec<NodeId>,
        tail: Option<NodeId>,
    },
    Quote(NodeId),
}
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub datum: Datum,
    pub span: Span,
}
#[derive(Clone, Debug)]
pub struct Form {
    pub(crate) source: Source,
    pub(crate) nodes: Rc<Vec<Node>>,
    pub(crate) root: NodeId,
    pub(crate) depth: usize,
}
impl Form {
    pub fn source(&self) -> &Source {
        &self.source
    }
    pub fn span(&self) -> Span {
        self.nodes[self.root].span
    }
}
#[derive(Debug)]
pub enum ReadState {
    Complete { form: Form, consumed: usize },
    More,
    End,
}
#[derive(Clone, Copy, Debug)]
pub struct Reader {
    pub input_limit: usize,
    pub depth_limit: usize,
}
impl Default for Reader {
    fn default() -> Self {
        Self {
            input_limit: 64 * 1024,
            depth_limit: 1024,
        }
    }
}
enum Frame {
    Quote(Span),
    List {
        start: Span,
        items: Vec<NodeId>,
        tail: Option<NodeId>,
        dot: bool,
        bytes: bool,
    },
}
impl Reader {
    pub fn read(&self, source: Source, eof: bool) -> Result<ReadState, Diagnostic> {
        let fail = |kind, message, span| Diagnostic::new(kind, message, &source, span);
        if source.text().len() > self.input_limit {
            return Err(fail(
                ErrorKind::ResourceLimit,
                "input limit exceeded",
                Span::default(),
            ));
        }
        let mut frames: Vec<Frame> = Vec::new();
        let mut nodes: Vec<Node> = Vec::new();
        let mut depth = 0;
        for (token, span) in lexer::scan(source.text()) {
            let token = token.map_err(|_| fail(ErrorKind::Lexical, "unrecognized token", span))?;
            if token == Token::String(false) {
                lexer::unfinished(&source, span)?;
                return if eof {
                    Err(fail(ErrorKind::Syntax, "unfinished string", span))
                } else {
                    Ok(ReadState::More)
                };
            }
            let datum = match token {
                Token::Open | Token::Bytes => {
                    if frames.len() >= self.depth_limit {
                        return Err(fail(
                            ErrorKind::ResourceLimit,
                            "reader depth exceeded",
                            span,
                        ));
                    }
                    frames.push(Frame::List {
                        start: span,
                        items: Vec::new(),
                        tail: None,
                        dot: false,
                        bytes: token == Token::Bytes,
                    });
                    depth = depth.max(frames.len());
                    continue;
                }
                Token::Quote => {
                    if frames.len() >= self.depth_limit {
                        return Err(fail(
                            ErrorKind::ResourceLimit,
                            "reader depth exceeded",
                            span,
                        ));
                    }
                    frames.push(Frame::Quote(span));
                    depth = depth.max(frames.len());
                    continue;
                }
                Token::Dot => {
                    match frames.last_mut() {
                        Some(Frame::List {
                            items,
                            dot,
                            bytes: false,
                            ..
                        }) if !items.is_empty() && !*dot => *dot = true,
                        _ => return Err(fail(ErrorKind::Syntax, "misplaced dot", span)),
                    }
                    continue;
                }
                Token::Shut => {
                    let Some(Frame::List {
                        start,
                        items,
                        tail,
                        dot,
                        bytes,
                    }) = frames.pop()
                    else {
                        return Err(fail(
                            ErrorKind::Syntax,
                            "unexpected closing parenthesis",
                            span,
                        ));
                    };
                    if dot && tail.is_none() {
                        return Err(fail(ErrorKind::Syntax, "missing dotted tail", span));
                    }
                    let datum = if bytes {
                        let mut out = Vec::new();
                        for &item in &items {
                            match nodes[item].datum {
                                Datum::Atom(Atom::Integer(n)) if (0..=255).contains(&n) => {
                                    out.push(n as u8)
                                }
                                _ => {
                                    return Err(fail(
                                        ErrorKind::Type,
                                        "byte literal requires integers from 0 to 255",
                                        nodes[item].span,
                                    ));
                                }
                            }
                        }
                        Datum::Atom(Atom::Bytes(out))
                    } else {
                        Datum::List { items, tail }
                    };
                    Node {
                        datum,
                        span: Span::new(start.start, span.end),
                    }
                }
                Token::True | Token::False => Node {
                    datum: Datum::Atom(Atom::Boolean(token == Token::True)),
                    span,
                },
                Token::Integer => {
                    let n = source.text()[span.start..span.end].parse().map_err(|_| {
                        fail(ErrorKind::Arithmetic, "integer outside i64 range", span)
                    })?;
                    Node {
                        datum: Datum::Atom(Atom::Integer(n)),
                        span,
                    }
                }
                Token::String(true) => Node {
                    datum: Datum::Atom(Atom::String(lexer::string(&source, span)?)),
                    span,
                },
                Token::Symbol => Node {
                    datum: Datum::Atom(Atom::Symbol(String::from(
                        &source.text()[span.start..span.end],
                    ))),
                    span,
                },
                Token::String(false) => unreachable!(),
            };
            let mut id = nodes.len();
            nodes.push(datum);
            loop {
                match frames.last_mut() {
                    Some(Frame::Quote(_)) => {
                        let Some(Frame::Quote(start)) = frames.pop() else {
                            unreachable!()
                        };
                        let end = nodes[id].span.end;
                        let quoted = id;
                        id = nodes.len();
                        nodes.push(Node {
                            datum: Datum::Quote(quoted),
                            span: Span::new(start.start, end),
                        });
                    }
                    Some(Frame::List {
                        items, tail, dot, ..
                    }) => {
                        if *dot {
                            if tail.replace(id).is_some() {
                                return Err(fail(
                                    ErrorKind::Syntax,
                                    "more than one dotted tail",
                                    span,
                                ));
                            }
                        } else {
                            items.push(id);
                        }
                        break;
                    }
                    None => {
                        return Ok(ReadState::Complete {
                            consumed: nodes[id].span.end,
                            form: Form {
                                source: source.clone(),
                                nodes: Rc::new(nodes),
                                root: id,
                                depth,
                            },
                        });
                    }
                }
            }
        }
        if frames.is_empty() {
            return Ok(ReadState::End);
        }
        if !eof {
            return Ok(ReadState::More);
        }
        Err(fail(
            ErrorKind::Syntax,
            "unfinished expression",
            Span::new(source.text().len(), source.text().len()),
        ))
    }
}

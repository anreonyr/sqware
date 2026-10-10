use super::reader::{Atom, Datum, Form, NodeId};
use crate::{Diagnostic, ErrorKind, Span};
use alloc::{rc::Rc, string::String, vec, vec::Vec};
pub(crate) type ExprId = usize;
#[derive(Clone, Debug)]
pub(crate) enum Binding {
    Local { depth: usize, slot: usize },
    Global(String),
}
#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Literal(Atom),
    Quote(NodeId),
    Reference(Binding),
    If {
        test: ExprId,
        yes: ExprId,
        no: ExprId,
    },
    Sequence(Vec<ExprId>),
    Lambda {
        params: usize,
        body: ExprId,
    },
    Call {
        function: ExprId,
        args: Vec<ExprId>,
    },
    Define {
        name: String,
        value: ExprId,
    },
    Assign {
        binding: Binding,
        value: ExprId,
    },
}
#[derive(Clone, Debug)]
pub(crate) struct Expression {
    pub expr: Expr,
    pub span: Span,
}
#[derive(Debug)]
pub(crate) struct Module {
    pub form: Form,
    pub expressions: Vec<Expression>,
    pub root: ExprId,
}
impl Module {
    pub fn weight(&self) -> usize {
        let syntax = self
            .form
            .nodes
            .iter()
            .map(|node| {
                core::mem::size_of::<super::reader::Node>()
                    + match &node.datum {
                        Datum::Atom(Atom::String(s) | Atom::Symbol(s)) => s.capacity(),
                        Datum::Atom(Atom::Bytes(b)) => b.capacity(),
                        Datum::List { items, .. } => {
                            items.capacity() * core::mem::size_of::<NodeId>()
                        }
                        _ => 0,
                    }
            })
            .sum::<usize>();
        let dynamic = self
            .expressions
            .iter()
            .map(|expression| match &expression.expr {
                Expr::Literal(Atom::String(s) | Atom::Symbol(s))
                | Expr::Define { name: s, .. }
                | Expr::Reference(Binding::Global(s))
                | Expr::Assign {
                    binding: Binding::Global(s),
                    ..
                } => s.capacity(),
                Expr::Literal(Atom::Bytes(b)) => b.capacity(),
                Expr::Sequence(parts) | Expr::Call { args: parts, .. } => {
                    parts.capacity() * core::mem::size_of::<ExprId>()
                }
                _ => 0,
            })
            .sum::<usize>();
        self.form.source.text().len()
            + syntax
            + dynamic
            + self.expressions.capacity() * core::mem::size_of::<Expression>()
    }
}
type Scope = Option<usize>;
struct ScopeFrame {
    parent: Scope,
    names: Vec<String>,
}
enum Build {
    If,
    Sequence(usize),
    Lambda(usize),
    Call(usize),
    Define(String),
    Assign(Binding),
    Let(usize),
}
enum Work {
    Visit(NodeId, Scope, bool),
    Build(Build, Span),
}
fn symbol(form: &Form, id: NodeId) -> Option<&str> {
    match &form.nodes[id].datum {
        Datum::Atom(Atom::Symbol(s)) => Some(s),
        _ => None,
    }
}
fn list(form: &Form, id: NodeId) -> Option<&[NodeId]> {
    match &form.nodes[id].datum {
        Datum::List { items, tail: None } => Some(items),
        _ => None,
    }
}
fn binding(name: &str, scopes: &[ScopeFrame], mut scope: Scope) -> Binding {
    let mut depth = 0;
    while let Some(index) = scope {
        let frame = &scopes[index];
        if let Some(slot) = frame.names.iter().position(|s| s == name) {
            return Binding::Local { depth, slot };
        }
        scope = frame.parent;
        depth += 1;
    }
    Binding::Global(String::from(name))
}

fn parameters(form: &Form, ids: &[NodeId]) -> Result<Vec<String>, Diagnostic> {
    let mut params = Vec::new();
    for &id in ids {
        let name = symbol(form, id).ok_or_else(|| {
            Diagnostic::new(
                ErrorKind::Syntax,
                "parameter must be a symbol",
                &form.source,
                form.nodes[id].span,
            )
        })?;
        if params.iter().any(|p| p == name) {
            return Err(Diagnostic::new(
                ErrorKind::Syntax,
                "duplicate parameter",
                &form.source,
                form.nodes[id].span,
            ));
        }
        params.push(String::from(name));
    }
    Ok(params)
}
fn push(expressions: &mut Vec<Expression>, expr: Expr, span: Span) -> ExprId {
    let id = expressions.len();
    expressions.push(Expression { expr, span });
    id
}
fn visits(work: &mut Vec<Work>, ids: &[NodeId], scope: Scope, top: bool) {
    for &id in ids.iter().rev() {
        work.push(Work::Visit(id, scope, top));
    }
}
pub(crate) fn lower(form: Form) -> Result<Rc<Module>, Diagnostic> {
    let mut work = vec![Work::Visit(form.root, None, true)];
    let mut scopes: Vec<ScopeFrame> = Vec::new();
    let mut results: Vec<ExprId> = Vec::new();
    let mut expressions = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            Work::Visit(id, scope, top) => {
                let node = &form.nodes[id];
                let span = node.span;
                let fail =
                    |message| Diagnostic::new(ErrorKind::Syntax, message, &form.source, span);
                match &node.datum {
                    Datum::Atom(Atom::Symbol(s)) => results.push(push(
                        &mut expressions,
                        Expr::Reference(binding(s, &scopes, scope)),
                        span,
                    )),
                    Datum::Atom(atom) => {
                        results.push(push(&mut expressions, Expr::Literal(atom.clone()), span))
                    }
                    Datum::Quote(quoted) => {
                        results.push(push(&mut expressions, Expr::Quote(*quoted), span))
                    }
                    Datum::List { items, tail } => {
                        if tail.is_some() {
                            return Err(fail("application must be a proper list"));
                        }
                        if items.is_empty() {
                            results.push(push(&mut expressions, Expr::Quote(id), span));
                            continue;
                        }
                        match symbol(&form, items[0]) {
                            Some("quote") => {
                                if items.len() != 2 {
                                    return Err(fail("quote takes one datum"));
                                }
                                results.push(push(&mut expressions, Expr::Quote(items[1]), span));
                            }
                            Some("if") => {
                                if items.len() != 4 {
                                    return Err(fail("if takes test, consequent and alternative"));
                                }
                                work.push(Work::Build(Build::If, span));
                                visits(&mut work, &items[1..], scope, false);
                            }
                            Some("begin") => {
                                work.push(Work::Build(Build::Sequence(items.len() - 1), span));
                                visits(&mut work, &items[1..], scope, top);
                            }
                            Some("lambda") => {
                                if items.len() < 3 {
                                    return Err(fail("lambda requires parameters and a body"));
                                }
                                let ids = list(&form, items[1]).ok_or_else(|| {
                                    fail("lambda parameters must be a proper list")
                                })?;
                                let params = parameters(&form, ids)?;
                                let n = params.len();
                                let inner = Some(scopes.len());
                                scopes.push(ScopeFrame {
                                    parent: scope,
                                    names: params,
                                });
                                work.push(Work::Build(Build::Lambda(n), span));
                                work.push(Work::Build(Build::Sequence(items.len() - 2), span));
                                visits(&mut work, &items[2..], inner, false);
                            }
                            Some("define") => {
                                if !top {
                                    return Err(fail("define is restricted to top level"));
                                }
                                if items.len() < 3 {
                                    return Err(fail("define requires a name and value"));
                                }
                                if let Some(name) = symbol(&form, items[1]) {
                                    if items.len() != 3 {
                                        return Err(fail("define takes one value"));
                                    }
                                    work.push(Work::Build(Build::Define(String::from(name)), span));
                                    work.push(Work::Visit(items[2], scope, false));
                                } else {
                                    let header = list(&form, items[1])
                                        .filter(|p| !p.is_empty())
                                        .ok_or_else(|| fail("invalid function definition"))?;
                                    let name = symbol(&form, header[0])
                                        .ok_or_else(|| fail("function name must be a symbol"))?;
                                    let params = parameters(&form, &header[1..])?;
                                    let n = params.len();
                                    let inner = Some(scopes.len());
                                    scopes.push(ScopeFrame {
                                        parent: scope,
                                        names: params,
                                    });
                                    work.push(Work::Build(Build::Define(String::from(name)), span));
                                    work.push(Work::Build(Build::Lambda(n), span));
                                    work.push(Work::Build(Build::Sequence(items.len() - 2), span));
                                    visits(&mut work, &items[2..], inner, false);
                                }
                            }
                            Some("set!") => {
                                if items.len() != 3 {
                                    return Err(fail("set! takes a name and value"));
                                }
                                let name = symbol(&form, items[1])
                                    .ok_or_else(|| fail("set! target must be a symbol"))?;
                                work.push(Work::Build(
                                    Build::Assign(binding(name, &scopes, scope)),
                                    span,
                                ));
                                work.push(Work::Visit(items[2], scope, false));
                            }
                            Some("let") => {
                                if items.len() < 3 {
                                    return Err(fail("let requires bindings and a body"));
                                }
                                let bindings = list(&form, items[1])
                                    .ok_or_else(|| fail("let bindings must be a proper list"))?;
                                let mut names = Vec::new();
                                let mut values = Vec::new();
                                for &entry in bindings {
                                    let pair =
                                        list(&form, entry).filter(|p| p.len() == 2).ok_or_else(
                                            || fail("let binding requires a name and value"),
                                        )?;
                                    names.push(pair[0]);
                                    values.push(pair[1]);
                                }
                                let params = parameters(&form, &names)?;
                                let inner = Some(scopes.len());
                                scopes.push(ScopeFrame {
                                    parent: scope,
                                    names: params,
                                });
                                work.push(Work::Build(Build::Let(bindings.len()), span));
                                work.push(Work::Build(Build::Sequence(items.len() - 2), span));
                                visits(&mut work, &items[2..], inner, false);
                                visits(&mut work, &values, scope, false);
                            }
                            _ => {
                                work.push(Work::Build(Build::Call(items.len() - 1), span));
                                visits(&mut work, items, scope, false);
                            }
                        }
                    }
                }
            }
            Work::Build(build, span) => {
                let count = match &build {
                    Build::If => 3,
                    Build::Sequence(n) => *n,
                    Build::Call(n) | Build::Let(n) => n + 1,
                    _ => 1,
                };
                let parts = results.split_off(results.len() - count);
                let expr = match build {
                    Build::If => Expr::If {
                        test: parts[0],
                        yes: parts[1],
                        no: parts[2],
                    },
                    Build::Sequence(_) => Expr::Sequence(parts),
                    Build::Lambda(params) => Expr::Lambda {
                        params,
                        body: parts[0],
                    },
                    Build::Call(_) => Expr::Call {
                        function: parts[0],
                        args: parts[1..].to_vec(),
                    },
                    Build::Define(name) => Expr::Define {
                        name,
                        value: parts[0],
                    },
                    Build::Assign(binding) => Expr::Assign {
                        binding,
                        value: parts[0],
                    },
                    Build::Let(params) => {
                        let function = push(
                            &mut expressions,
                            Expr::Lambda {
                                params,
                                body: parts[params],
                            },
                            span,
                        );
                        Expr::Call {
                            function,
                            args: parts[..params].to_vec(),
                        }
                    }
                };
                results.push(push(&mut expressions, expr, span));
            }
        }
    }
    Ok(Rc::new(Module {
        form,
        expressions,
        root: results[0],
    }))
}

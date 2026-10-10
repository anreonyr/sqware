use lisp::{
    Arity, Engine, ErrorKind, ForeignId, HostError, Limits, ReadState, Reader, RootValue, Source,
    Step, ValueKind,
};
fn engine() -> Engine {
    Engine::new(Limits::default()).unwrap()
}
fn start(engine: &mut Engine, source: &str) {
    let ReadState::Complete { form, .. } = Reader::default()
        .read(Source::new("test", source), true)
        .unwrap()
    else {
        panic!("expected form")
    };
    engine.start(form).unwrap();
}
fn eval(engine: &mut Engine, source: &str) -> Result<RootValue, lisp::Diagnostic> {
    start(engine, source);
    loop {
        match engine.step(1024) {
            Step::Done(value) => return Ok(value),
            Step::Failed(error) => return Err(error),
            Step::Yielded => {}
            Step::Request(_) => panic!("unexpected native request"),
        }
    }
}
#[test]
fn lexer_reader_spans_and_incomplete_input() {
    let reader = Reader::default();
    for input in ["(a", "'", "\"unfinished", "#u8(1"] {
        assert!(matches!(
            reader.read(Source::new("repl", input), false).unwrap(),
            ReadState::More
        ));
        assert_eq!(
            reader
                .read(Source::new("repl", input), true)
                .unwrap_err()
                .kind,
            ErrorKind::Syntax
        );
    }
    for input in [")", "(. a)", "(a . b c)", "(a .)", "\"\\q\"", "#u8(256)"] {
        assert!(
            reader.read(Source::new("bad", input), true).is_err(),
            "{input}"
        );
    }
    let ReadState::Complete { form, consumed } = reader
        .read(Source::new("test", "; comment\n'中 rest"), true)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(consumed, 14);
    assert_eq!(form.source().location(form.span().start), (2, 1));
}
#[test]
fn reader_limits_are_errors() {
    let reader = Reader {
        input_limit: 20,
        depth_limit: 3,
    };
    assert_eq!(
        reader
            .read(Source::new("deep", "((((1))))"), true)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        reader
            .read(Source::new("long", &"a".repeat(21)), true)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn language_values_and_printing_round_trip() {
    let mut engine = engine();
    for source in [
        "'()",
        "'(a 1 . b)",
        "'(a (b c))",
        "#u8(0 255 32)",
        "\"中\\n\\\"\\\\\"",
        "''x",
    ] {
        let value = eval(&mut engine, source).unwrap();
        let printed = engine.display(&value).unwrap();
        let expression = if matches!(
            engine.kind(&value).unwrap(),
            ValueKind::Pair | ValueKind::Symbol | ValueKind::Nil
        ) {
            format!("'{printed}")
        } else {
            printed.clone()
        };
        let second = eval(&mut engine, &expression).unwrap();
        assert_eq!(engine.display(&second).unwrap(), printed);
    }
}
#[test]
fn arithmetic_truth_and_errors() {
    let mut engine = engine();
    assert_eq!(
        eval(&mut engine, "(+ 1 (* 2 3) (- 9 4))")
            .unwrap()
            .integer(),
        Some(12)
    );
    assert_eq!(
        eval(&mut engine, "(if '() 1 2)").unwrap().integer(),
        Some(1)
    );
    assert_eq!(eval(&mut engine, "(if #f 1 2)").unwrap().integer(), Some(2));
    for (input, kind) in [
        ("(/ 1 0)", ErrorKind::Arithmetic),
        ("(+ 9223372036854775807 1)", ErrorKind::Arithmetic),
        ("(car 1)", ErrorKind::Type),
        ("(cons 1)", ErrorKind::Arity),
        ("missing", ErrorKind::Unbound),
    ] {
        assert_eq!(eval(&mut engine, input).unwrap_err().kind, kind);
        assert_eq!(eval(&mut engine, "(+ 1 2)").unwrap().integer(), Some(3));
    }
}
#[test]
fn lexical_capture_assignment_and_parallel_let() {
    let mut engine = engine();
    let value = eval(&mut engine, "(begin (define x 7) (define f (let ((x 10)) (lambda (y) (begin (set! x (+ x y)) x)))) (f 2) (f 3))").unwrap();
    assert_eq!(value.integer(), Some(15));
    assert_eq!(eval(&mut engine, "x").unwrap().integer(), Some(7));
    assert_eq!(
        eval(&mut engine, "(let ((x 1) (y x)) y)")
            .unwrap()
            .integer(),
        Some(7)
    );
    assert_eq!(
        eval(
            &mut engine,
            "(begin (define n 0) ((lambda (a b) n) (set! n 1) (set! n 2)))"
        )
        .unwrap()
        .integer(),
        Some(2)
    );
    assert_eq!(
        eval(&mut engine, "(let ((x 1)) ((lambda (y) (+ x y)) 2))")
            .unwrap()
            .integer(),
        Some(3)
    );
}
#[test]
fn lowering_checks_special_forms_before_execution() {
    let mut engine = engine();
    for source in [
        "(lambda (x x) x)",
        "(let ((x 1) (x 2)) x)",
        "(let ((x 1)) (define y 2))",
        "(set! 1 2)",
        "(if #t 1)",
    ] {
        let ReadState::Complete { form, .. } = Reader::default()
            .read(Source::new("bad", source), true)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(engine.start(form).unwrap_err().kind, ErrorKind::Syntax);
    }
}
#[test]
fn hundred_thousand_tail_calls_have_bounded_frames() {
    let mut engine = engine();
    let value = eval(
        &mut engine,
        "(begin (define (loop n sum) (if (= n 0) sum (loop (- n 1) (+ sum 1)))) (loop 100000 0))",
    )
    .unwrap();
    assert_eq!(value.integer(), Some(100000));
    assert!(engine.peak_frames() < 12, "{}", engine.peak_frames());
    engine.collect();
    assert!(engine.heap_used() < 32000);
}
#[test]
fn non_tail_recursion_limit_is_recoverable() {
    let mut engine = Engine::new(Limits {
        frames: 40,
        ..Limits::default()
    })
    .unwrap();
    assert_eq!(
        eval(
            &mut engine,
            "(begin (define (loop n) (if (= n 0) 0 (+ 1 (loop (- n 1))))) (loop 1000))"
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(eval(&mut engine, "(loop 3)").unwrap().integer(), Some(3));
}
#[test]
fn cyclic_closure_environment_is_collected() {
    let mut engine = engine();
    eval(&mut engine, "(define keep #f)").unwrap();
    engine.collect();
    let baseline = engine.heap_used();
    let value = eval(
        &mut engine,
        "(let ((f #f)) (begin (set! f (lambda () f)) (set! keep f)))",
    )
    .unwrap();
    engine.collect();
    assert!(engine.heap_used() > baseline);
    drop(value);
    eval(&mut engine, "(set! keep #f)").unwrap();
    engine.collect();
    assert_eq!(engine.heap_used(), baseline);
}
#[test]
fn host_calls_root_arguments_resume_and_reject_late_results() {
    let mut engine = engine();
    engine.register("host", 17, Arity::fixed(1)).unwrap();
    start(&mut engine, "(+ 1 (host (list 2 3)))");
    let call = loop {
        match engine.step(2) {
            Step::Request(call) => break call,
            Step::Yielded => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(call.operation, 17);
    engine.collect();
    assert_eq!(engine.display(&call.arguments[0]).unwrap(), "(2 3)");
    assert!(matches!(engine.step(1024), Step::Yielded));
    engine.resume(call.id, Ok(engine.integer(41))).unwrap();
    let result = loop {
        match engine.step(1024) {
            Step::Done(value) => break value,
            Step::Yielded => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(result.integer(), Some(42));
    start(&mut engine, "(host 1)");
    let pending = loop {
        if let Step::Request(call) = engine.step(1024) {
            break call;
        }
    };
    engine.cancel();
    assert_eq!(
        engine
            .resume(pending.id, Ok(engine.nil()))
            .unwrap_err()
            .kind,
        ErrorKind::StaleCall
    );
    assert_eq!(eval(&mut engine, "9").unwrap().integer(), Some(9));
}
#[test]
fn host_error_and_foreign_reclamation() {
    let mut engine = engine();
    engine.register("host", 0, Arity::fixed(1)).unwrap();
    let id = ForeignId {
        slot: 7,
        generation: 2,
    };
    let foreign = engine.foreign(id, "port").unwrap();
    let alias = foreign.clone();
    engine.collect();
    assert!(engine.take_released().is_empty());
    assert_eq!(engine.display(&foreign).unwrap(), "<port>");
    drop(foreign);
    engine.collect();
    assert!(engine.take_released().is_empty());
    drop(alias);
    engine.collect();
    assert_eq!(engine.take_released(), vec![id]);
    start(&mut engine, "(host 1)");
    let call = loop {
        if let Step::Request(call) = engine.step(1024) {
            break call;
        }
    };
    assert_eq!(
        engine
            .resume(call.id, Err(HostError("broken pipe".into())))
            .unwrap_err()
            .kind,
        ErrorKind::Host
    );
    assert_eq!(eval(&mut engine, "1").unwrap().integer(), Some(1));
}
#[test]
fn cross_engine_values_are_rejected() {
    let mut a = engine();
    let b = engine();
    a.register("host", 0, Arity::fixed(0)).unwrap();
    start(&mut a, "(host)");
    let call = loop {
        if let Step::Request(call) = a.step(1024) {
            break call;
        }
    };
    assert_eq!(
        a.resume(call.id, Ok(b.integer(1))).unwrap_err().kind,
        ErrorKind::StaleValue
    );
    a.resume(call.id, Ok(a.integer(2))).unwrap();
}
#[test]
fn cancelled_infinite_evaluation_keeps_globals() {
    let mut engine = engine();
    start(&mut engine, "(begin (define (spin) (spin)) (spin))");
    for _ in 0..10 {
        assert!(matches!(engine.step(1024), Step::Yielded));
    }
    engine.cancel();
    assert_eq!(eval(&mut engine, "(+ 3 4)").unwrap().integer(), Some(7));
}

#[test]
fn reader_commits_one_form_before_later_incomplete_or_invalid_input() {
    let reader = Reader::default();
    for input in ["(define x 1) \"unfinished", "(define x 1) )"] {
        let ReadState::Complete { consumed, .. } =
            reader.read(Source::new("repl", input), false).unwrap()
        else {
            panic!()
        };
        assert_eq!(consumed, 12);
    }
}
#[test]
fn deeply_nested_lexical_scopes_do_not_use_the_rust_stack() {
    let mut source = "(let ((x 5)) ".to_string();
    for _ in 0..250 {
        source.push_str("(let ((y 1)) ");
    }
    source.push('x');
    for _ in 0..251 {
        source.push(')');
    }
    let mut engine = engine();
    assert_eq!(eval(&mut engine, &source).unwrap().integer(), Some(5));
}

#[test]
fn long_value_operations_yield_and_survive_collection() {
    let mut engine = engine();
    let quoted = format!("'({})", "1 ".repeat(6000));
    start(&mut engine, &quoted);
    assert!(matches!(engine.step(1024), Step::Yielded));
    engine.collect();
    let value = loop {
        match engine.step(1024) {
            Step::Done(value) => break value,
            Step::Yielded => engine.collect(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(engine.display(&value).unwrap().matches('1').count(), 6000);
    let text = "中".repeat(5000);
    eval(&mut engine, &format!("(define text \"{text}\")")).unwrap();
    start(&mut engine, "(string->bytes text)");
    assert!(matches!(engine.step(8), Step::Yielded));
    engine.collect();
    let bytes = loop {
        match engine.step(8) {
            Step::Done(value) => break value,
            Step::Yielded => engine.collect(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(engine.byte_slice(&bytes).unwrap().unwrap(), text.as_bytes());
    start(&mut engine, "(string-append text text)");
    assert!(matches!(engine.step(8), Step::Yielded));
    engine.cancel();
    engine.collect();
    assert_eq!(
        eval(&mut engine, "(string-length text)").unwrap().integer(),
        Some(5000)
    );
}

#[test]
fn engine_limits_cannot_be_bypassed_by_a_more_permissive_reader() {
    let reader = Reader {
        input_limit: 65536,
        depth_limit: 1024,
    };
    let ReadState::Complete { form, .. } =
        reader.read(Source::new("deep", "'(((1)))"), true).unwrap()
    else {
        panic!()
    };
    let mut engine = Engine::new(Limits {
        depth: 2,
        ..Limits::default()
    })
    .unwrap();
    assert_eq!(
        engine.start(form).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        Reader::default()
            .read(Source::new("invalid", "\"\\q"), false)
            .unwrap_err()
            .kind,
        ErrorKind::Lexical
    );
}
#[test]
fn global_names_are_charged_and_heap_limit_error_is_recoverable() {
    let mut engine = Engine::new(Limits {
        heap: 8192,
        ..Limits::default()
    })
    .unwrap();
    let mut limited = false;
    for index in 0..100 {
        match eval(
            &mut engine,
            &format!("(define variable_{index}_with_a_long_name {index})"),
        ) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::ResourceLimit);
                limited = true;
                break;
            }
        }
    }
    assert!(limited);
    assert_eq!(eval(&mut engine, "(+ 2 3)").unwrap().integer(), Some(5));
}

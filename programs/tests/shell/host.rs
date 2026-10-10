use crate::{adapt::controller::Controller, native::host::Host};
use lisp::{Engine, Limits, ReadState, Reader, RootValue, Source, Step};
fn setup() -> (Engine, Host) {
    let mut engine = Engine::new(Limits::default()).unwrap();
    Host::install(&mut engine).unwrap();
    (engine, Host::new(Controller::new()))
}
fn start(engine: &mut Engine, source: &str) {
    let ReadState::Complete { form, .. } = Reader::default()
        .read(Source::new("host-test", source), true)
        .unwrap()
    else {
        panic!()
    };
    engine.start(form).unwrap();
}
fn evaluate(engine: &mut Engine, host: &mut Host, source: &str) -> Result<RootValue, String> {
    start(engine, source);
    for _ in 0..10000 {
        host.advance(engine).map_err(|e| e.render())?;
        match engine.step(32) {
            Step::Done(value) => return Ok(value),
            Step::Failed(error) => return Err(error.render()),
            Step::Request(call) => host.request(engine, call).map_err(|e| e.render())?,
            Step::Yielded => {}
        }
    }
    panic!("mock exchange did not complete")
}
#[test]
fn production_host_checks_values_and_alias_close_and_keeps_plan_roots() {
    let (mut engine, mut host) = setup();
    evaluate(
        &mut engine,
        &mut host,
        "(define p (buffer 'read #u8(65 0 255)))",
    )
    .unwrap();
    let value = evaluate(&mut engine, &mut host, "(read p 2)").unwrap();
    assert_eq!(engine.display(&value).unwrap(), "(data #u8(65 0))");
    for source in [
        "(read 1)",
        "(write p #u8(1))",
        "(command 'mock '(1))",
        "(connect (list (command 'mock '())) (list (list p p)))",
    ] {
        assert!(evaluate(&mut engine, &mut host, source).is_err());
    }
    evaluate(&mut engine, &mut host, "(define alias p)").unwrap();
    evaluate(&mut engine, &mut host, "(close alias)").unwrap();
    assert!(evaluate(&mut engine, &mut host, "(read p)").is_err());
    evaluate(&mut engine, &mut host, "(close p)").unwrap();
    evaluate(
        &mut engine,
        &mut host,
        "(define plan (connect (list (command 'mock '())) '()))",
    )
    .unwrap();
    engine.collect();
    host.advance(&mut engine).unwrap();
    evaluate(&mut engine, &mut host, "(define job (prepare plan))").unwrap();
    assert_eq!(host.controller.jobs().count(), 1);
}
#[test]
fn waiting_evaluation_allows_control_progress_and_returns_all_member_results() {
    let (mut engine, mut host) = setup();
    evaluate(
        &mut engine,
        &mut host,
        "(define job (spawn (connect (list (command 'mock '()) (command 'mock '())) '())))",
    )
    .unwrap();
    start(&mut engine, "(wait job)");
    let Step::Request(call) = engine.step(32) else {
        panic!()
    };
    host.request(&mut engine, call).unwrap();
    let before = host.controller.events;
    for _ in 0..12 {
        host.advance(&mut engine).unwrap();
        assert!(matches!(engine.step(32), Step::Yielded));
    }
    assert!(host.controller.events >= before + 12);
    host.controller.finish = true;
    let value = loop {
        host.advance(&mut engine).unwrap();
        if let Step::Done(value) = engine.step(32) {
            break value;
        }
    };
    assert_eq!(engine.display(&value).unwrap(), "(completed (0 17))");
}
#[test]
fn cancelling_pending_prepare_rolls_back_and_later_evaluation_recovers() {
    let (mut engine, mut host) = setup();
    start(
        &mut engine,
        "(prepare (connect (list (command 'mock '())) '()))",
    );
    loop {
        match engine.step(32) {
            Step::Request(call) => {
                let is_prepare = call.operation == 9;
                host.request(&mut engine, call).unwrap();
                if is_prepare {
                    break;
                }
            }
            Step::Yielded => {}
            other => panic!("{other:?}"),
        }
    }
    host.cancel_evaluation(&mut engine);
    for _ in 0..12 {
        host.advance(&mut engine).unwrap();
    }
    assert!(
        host.controller
            .jobs()
            .all(|id| host.controller.job(id).unwrap().model.status.terminal())
    );
    assert_eq!(
        evaluate(&mut engine, &mut host, "(+ 2 3)")
            .unwrap()
            .integer(),
        Some(5)
    );
}

#[test]
fn retained_external_commands_hit_a_limit_without_poisoning_evaluation() {
    let (mut engine, mut host) = setup();
    let text = "x".repeat(32768);
    evaluate(
        &mut engine,
        &mut host,
        &format!("(define argument \"{text}\")"),
    )
    .unwrap();
    evaluate(&mut engine, &mut host, "(define kept '())").unwrap();
    let mut limited = false;
    for _ in 0..140 {
        match evaluate(
            &mut engine,
            &mut host,
            "(set! kept (cons (command 'mock (list argument)) kept))",
        ) {
            Ok(_) => {}
            Err(error) => {
                assert!(error.contains("memory limit"), "{error}");
                limited = true;
                break;
            }
        }
    }
    assert!(limited);
    assert_eq!(
        evaluate(&mut engine, &mut host, "(+ 1 2)")
            .unwrap()
            .integer(),
        Some(3)
    );
}

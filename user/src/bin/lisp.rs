#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;

use user::env::io::put;
use user::lisp::{Core, repl};

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("lisp\n");
    let mut core = Core::new();
    selftest(&mut core);
    repl(&mut core)
}

fn selftest(core: &mut Core) {
    for (src, expect) in [
        ("(+ 1 2)", "3"),
        ("(* 6 7)", "42"),
        ("(- 10 4)", "6"),
        ("(/ 7 2)", "3"),
        ("(car (quote (1 2 3)))", "1"),
        ("(cdr (quote (1 2 3)))", "(2 3)"),
        ("(cons 1 (quote (2 3)))", "(1 2 3)"),
        ("(eq? 1 1)", "t"),
        ("(eq? 1 2)", "()"),
        ("(quote (a b))", "(a b)"),
        ("(if t 7 8)", "7"),
        ("(if nil 7 8)", "8"),
        (
            "(define fact (lambda (n) (if (eq? n 0) 1 (* n (fact (- n 1))))))",
            "()",
        ),
        ("(fact 5)", "120"),
        ("(fact 0)", "1"),
    ] {
        let report = match core.read(src.as_bytes()) {
            Err(e) => format!("parse FAIL {src}: {e:?}"),
            Ok(v) => match core.eval(v) {
                Err(e) => format!("eval FAIL {src}: {e:?}"),
                Ok(r) => {
                    let s = core.print(&r);
                    if s == expect {
                        "ok".into()
                    } else {
                        format!("FAIL {src} -> {s} (want {expect})")
                    }
                }
            },
        };
        let _ = put(&report);
        let _ = put("\n");
    }
    let _ = put("selftest done\n");
}

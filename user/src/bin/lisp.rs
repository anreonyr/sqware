#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;

use user::env;
use user::lisp::{Core, repl};

// lisp：教学 Lisp shell——独占控制台输入（唯一 get 消费者），REPL 常驻直到
// (exit) 或 Ctrl+D。语言子集（无闭包）：整数/符号/nil/t + quote/car/cdr/cons/
// eq?/+/-/*/(整除)/if/define/lambda + 递归。
//
// 先跑语言自测（不依赖控制台输入，冒烟可验证 read/eval/print 链路），
// 再进入 repl。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    env::put("lisp\n").ok();
    let mut core = Core::new();
    selftest(&mut core);
    repl(&mut core)
}

/// 语言自测：求值一组表达式，与期望文本比对；逐项打 "ok"（失败打 FAIL 明细）。
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
        let _ = env::put(&report);
        let _ = env::put("\n");
    }
    let _ = env::put("selftest done\n");
}
//! REPL：经 Terminal 读行、解释执行，常驻循环。

use alloc::format;

use crate::env::room;
use crate::term::{Readline, Terminal};

use super::core::Core;
use super::kernel::{LispError, Val};

pub fn repl(core: &mut Core, term: &Terminal) -> ! {
    loop {
        let _ = crate::env::io::put("> ");
        let line = match term.readline() {
            Readline::Line(s) => s,
            Readline::Interrupt => continue,
            Readline::Eof => room::exit(),
        };
        let line = line.into_bytes();
        if line.iter().all(|b| matches!(b, b' ' | b'\t')) {
            continue;
        }
        match core.read(&line) {
            Err(e) => {
                let _ = crate::env::io::put(&format!("parse error({e:?}): {line:02x?}\n"));
            }
            Ok(v) => {
                if is_command(core, &v, "exit") {
                    room::exit();
                }
                let defined = is_command(core, &v, "define");
                match core.eval(v) {
                    Err(e) => err_line(e),
                    Ok(r) if !defined => {
                        let _ = crate::env::io::put(&core.print(&r));
                        let _ = crate::env::io::put("\n");
                    }
                    Ok(_) => {}
                }
            }
        }
    }
}

fn is_command(core: &Core, v: &Val, name: &str) -> bool {
    match v {
        Val::Cons(b) => matches!(&b.0, Val::Sym(s) if *s == core.bi(name)),
        _ => false,
    }
}

fn err_line(e: LispError) {
    let msg = match e {
        LispError::Parse => "parse error",
        LispError::Unbound => "unbound symbol",
        LispError::Arity => "wrong arity",
        LispError::BadForm => "bad form",
        LispError::NotCallable => "not callable",
    };
    let _ = crate::env::io::put(msg);
    let _ = crate::env::io::put("\n");
}

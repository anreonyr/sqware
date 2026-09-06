//! REPL：经 Terminal 读行、解释执行，常驻循环。Terminal 是唯一 console 出口。

use alloc::format;

use crate::env::room;
use crate::term::{Readline, Terminal};

use super::core::Core;
use super::kernel::{LispError, Val};

pub fn repl(core: &mut Core, term: &Terminal) -> ! {
    loop {
        term.write("> ");
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
                term.writeline(&format!("parse error({e:?}): {line:02x?}"));
            }
            Ok(v) => {
                if is_command(core, &v, "exit") {
                    room::exit();
                }
                let defined = is_command(core, &v, "define");
                match core.eval(v) {
                    Err(e) => err_line(term, e),
                    Ok(r) if !defined => {
                        term.writeline(&core.print(&r));
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

fn err_line(term: &Terminal, e: LispError) {
    let msg = match e {
        LispError::Parse => "parse error",
        LispError::Unbound => "unbound symbol",
        LispError::Arity => "wrong arity",
        LispError::BadForm => "bad form",
        LispError::NotCallable => "not callable",
    };
    term.writeline(msg);
}

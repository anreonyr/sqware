//! REPL：独占控制台输入，常驻循环。

use alloc::format;
use alloc::vec::Vec;

use crate::env::{io, room};

use super::core::Core;
use super::kernel::{LispError, Val};

pub fn repl(core: &mut Core) -> ! {
    loop {
        let _ = io::put("> ");
        let mut line = Vec::new();
        loop {
            let b = io::get();
            match b {
                0x0a | 0x0d => break,
                0x08 | 0x7f => {
                    line.pop();
                }
                0x03 => line.clear(),
                0x04 => room::exit(),
                b => line.push(b),
            }
        }
        if line.iter().all(|b| matches!(b, b' ' | b'\t')) {
            continue;
        }
        match core.read(&line) {
            Err(e) => {
                let _ = io::put(&format!("parse error({e:?}): {line:02x?}\n"));
            }
            Ok(v) => {
                if is_command(core, &v, "exit") {
                    room::exit();
                }
                let defined = is_command(core, &v, "define");
                match core.eval(v) {
                    Err(e) => err_line(e),
                    Ok(r) if !defined => {
                        let _ = io::put(&core.print(&r));
                        let _ = io::put("\n");
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
    let _ = io::put(msg);
    let _ = io::put("\n");
}

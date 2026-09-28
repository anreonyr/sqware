#![cfg(debug_assertions)]
use core::fmt;

#[macro_export]
macro_rules! expect {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            panic!("[health] {}", format_args!($($arg)*));
        }
    };
    ($cond:expr) => {
        $crate::expect!($cond, "expectation failed")
    };
}

#[cfg(debug_assertions)]
#[allow(dead_code)]
pub(crate) fn report_ok(item: &str, detail: fmt::Arguments) {
    crate::putln!("[health] {item}: ok ({detail})");
}

pub mod hart;
pub mod pagetable;
pub mod permit;
pub mod shell;
pub mod spare;
pub mod stress;

pub fn run() {
    #[cfg(debug_assertions)]
    {
        spare::accept();
        pagetable::pagetable();
        stress::accept();
        shell::accept();
        permit::form();
        permit::members();
        permit::fanout();
        permit::order();
        permit::badge();
        hart::count();
    }
}
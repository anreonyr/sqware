#![no_std]
#![no_main]

extern crate alloc;

use user::env::io::put;

// porter: port 内核邮路 demo——可改写为 Pie<Hole> 全链路（待续）
#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("porter: see Hole (env::mail) for port-equivalent\n");
}
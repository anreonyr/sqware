#![no_std]
#![no_main]

extern crate alloc;

use user::core::mail::{MSG_LEN, Port};
use user::core::task;
use user::env::{io::put, room::exit};

// mailer：port 内核邮路全链路——子任务 push 10 轮，主任务 pull 10 轮校验。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let _ = put("mailer\n");
    let port = Port::open().expect("port open");

    let child_port = port.clone();
    let _child = task::closure(move || {
        for i in 0..10u8 {
            let mut msg = [0u8; MSG_LEN];
            msg[0] = i;
            child_port.push(&msg).expect("push failed");
        }
    });

    for i in 0..10u8 {
        let mut buf = [0u8; MSG_LEN];
        port.pull(&mut buf).expect("pull failed");
        if buf[0] == i {
            let _ = put("P\n");
        }
    }

    exit()
}
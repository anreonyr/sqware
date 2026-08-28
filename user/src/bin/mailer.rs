#![no_std]
#![no_main]

extern crate alloc;

use user::env::{self, put};
use user::mail::{MSG_LEN, Port};
use user::task;

// mailer：port 内核邮路全链路——子任务 push 10 轮序号消息（槽满 → wait 阻塞），
// 主任务 pull 10 轮校验序号（槽空 → wait 阻塞），往返成功打 'P'。验证
// open/push/pull 阻塞语义 + 条件变更方 wake + 调度域 wait/wake 整链。

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    put("mailer\n").ok();
    let port = Port::open().expect("port open");

    // 子任务：push 10 轮序号消息（每轮 push 存入槽即返回；第二次起若槽未空
    // 则由条件循环 wait 阻塞到主任务 pull 取走）。
    let _child = task::closure(move || {
        for i in 0..10u8 {
            let mut msg = [0u8; MSG_LEN];
            msg[0] = i;
            port.push(&msg).expect("push failed");
        }
    });

    // 主任务：pull 10 轮校验（槽空 → wait 阻塞到子任务投递）。
    for i in 0..10u8 {
        let mut buf = [0u8; MSG_LEN];
        port.pull(&mut buf).expect("pull failed");
        if buf[0] == i {
            put("P\n").ok();
        }
    }

    port.shut().expect("shut failed");
    env::exit()
}

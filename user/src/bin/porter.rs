#![no_std]
#![no_main]

extern crate alloc;

use user::core::mail::{MSG_LEN, Port};
use user::core::task;
use user::env::io::put;

// porter：port 内核邮路全链路——主任务建 port，子任务 join 持生产端 push 10 轮，
// 主任务 pull 10 轮校验。port 入 task.mail：两端各持一份（Arc 保活），最后一份
// drop 置 Dead——无 race，无需 child.join() 同步。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("porter\n");
    let port = Port::open().expect("port open");

    let handle = port.handle();
    let _child = task::closure(move || {
        // 关键：子任务 join 同一句柄 → 本方 mail 多一份 Port → 末位 drop 守门
        // Arc 计数：父不先 drop（mail 持有至 reap），子 push 期间父句柄仍 Live。
        let child_port = Port::join(handle).expect("port join");
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
}

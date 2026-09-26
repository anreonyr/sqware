//! uart::adapt::resident — **常驻（起手 6）· 壳**：那条线一响就排空、交出去、说一句"我排空了"。
//!
//! 判定在 [`crate::core::batch`]（纯）：本文件只做"等、排空、交、说"——那条线是内核的，
//! `RBR`/`LSR` 是设备面的。
//!
//! **顺序是有意的**：**先读走设备里的字节，再说"排空了"**——路由者收到那句话才把线放回
//! （`exhaust` 是一个事件，不是节拍；而它不阻塞，见 `line::client::Line::exhaust`）。
//! 反过来的话，线放回了而字节还挂在设备里，就是"电平一直高、却没人读"的空转。

use super::boot::Up;
use super::fail;
use crate::core::batch::Batch;
use crate::uart as device;
use env::Wait;
use protocol::driver::line;
use runtime::env::mail::HolePie;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里，
/// **下一次中断（本域说"排空了" ⇒ 路由者放回线）再来**。
const DRAIN_MAX: usize = 64;

/// 常驻。
///
/// 失败：那条线没了（`receive` 答错）⇒ 这个域没有可继续的状态（照实报 `Dead`）。
pub fn run(up: &Up, held: line::client::Line) -> Result<(), fail::Fail> {
    // 读行那枚孔**就是门牌**：本域铸的，客人经树上 `FIND /device/uart` 拿到它的副本。
    let console = HolePie::from_token(up.entry);
    let mut raw = [0u8; DRAIN_MAX];
    loop {
        if held.receive(Wait::Forever).is_err() {
            return Err(fail::Fail::Dead);
        }
        let n = device::drain(up.view(), &mut raw);
        // 交给读行的人。**这一手要阻塞**：字节是内容，丢了补不回来；读行的人（`echo`）
        // 总会回到"取一行"那一格，故等它是有界的。
        //
        // **`n == 0` 那一趟不推**：[`Batch::of`] 把那一格做进了类型（内核只收 `1..=一页`）。
        // **实测**：少了这个判据（改成无条件 `push`）第一次空排空当场把本域打死
        // （`MailFail::Denied`），整台机器随之级联收场。
        if let Some(batch) = Batch::of(&raw, n) {
            console.push(batch.bytes()).unwrap();
        }
        // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 + 把线放回去。
        held.exhaust().unwrap();
        // debug!("uart: rang n={n}");
    }
}

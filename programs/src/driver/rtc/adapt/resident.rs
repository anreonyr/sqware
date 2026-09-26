//! rtc::adapt::resident — **常驻（起手 5）· 壳**：一只组等两个源，喂事件、执行动作。
//!
//! 判定在 [`Host`]（纯，见 `core/host.rs`）：本文件只做"等、取、喂、执行"——组与泊位是内核的，
//! 设备的读与清是设备面的。[`Host::ask`] / [`Host::ring`] 吐什么，这里就执行什么。

use super::boot::Up;
use super::desk;
use super::fail::{Fail, Step};
use crate::rtc;
use env::{HoleDir, Wait};
use programs::driver::rtc::core::frame::Time;
use programs::driver::rtc::core::host::{Host, Ring};
use protocol::debug;
use protocol::driver::line;
use protocol::session::slip::Slip;
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};

/// 常驻：**一只组等两个源**——门上有请求、线上有投递。
///
/// 两个源都是**事件**：请求是客人推来的，投递是设备自己拉线换来的，故等待没有期限。
/// 那只组的成员就是那两枚孔（"就绪"挂进组，"取消息"仍走各自那一手）。
///
/// 失败：组坏了 ⇒ `Err(Fail::at(Step::Desk))`——本域没有可继续的状态。
pub fn run(up: &Up, held: line::client::Line, host: &mut Host) -> Result<(), Fail> {
    let pile = Pile::unseal(false).map_err(|_| Fail::at(Step::Desk))?;
    let entry_hole = HolePie::from_token(up.entry);
    let lane = held.hole().map_err(|_| Fail::at(Step::Line))?;
    if pile.attach(&entry_hole, HoleDir::Pull).is_err()
        || pile
            .attach(&HolePie::from_token(lane), HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(Step::Desk));
    }

    let view = up.view();
    // 一问最长那一形是 `Arm`（`Now` 更短，也走得进来）；缓冲给**一页**（载体的界，
    // 见 `Push` 的前置条件）——于是任何一条消息一趟都取得出来。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(Step::Desk));
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        // 等到有事件。非阻塞地把两个源各取干净——**先门后线**：门上的问要就地答，而线那一趟
        // 到点才有的说（次序不承担语义，只省一次绕回）。
        match pile.await_(Wait::Forever) {
            Ok(_) => {}
            Err(_) => return Err(Fail::at(Step::Desk)),
        }
        // 门牌是**单槽**：一趟把槽里的都取走。缓冲是一页（见上），故"取不出也丢不掉"
        // 那个状态不存在。
        while let Ok((len, from)) = entry_hole.pull_timeout_from(&mut buf, Wait::POLL) {
            desk::serve(host, view, from, &buf[..len]);
        }
        while held.receive(Wait::POLL).is_ok() {
            // 一次投递 = 设备那一格拉起来了。顺序与 `uart` 同一条道理：**先把设备那一格清干净**
            // （清 `irq_pending`：电平源，不清线就一直挂着），再看那一格到点没有，最后说"排空了"。
            let now = rtc::now(view);
            rtc::clear(view);
            if let Ring::Rang { back, now } = host.ring(now) {
                // 那一声**上船台**（答那一形：一个时刻）——与客人收它走的是同一张表。
                match Slip::<Time>::seal(back)
                    .load(Time::of(now))
                    .ok()
                    .map(|s| s.ship())
                {
                    Some(Ok(())) => debug!("rtc: rang n={} now={now}", host.heard()),
                    // **推不出去 = 那位客人没了**（它开的那枚孔随它退场封印）。那一格已经空着
                    // （取走就是兑现），故这里只报一行，不重试、不补发——**读数也不加一**。
                    _ => debug!("rtc: notify failed"),
                }
                let _ = mail::release(back);
            }
            let _ = held.exhaust();
        }
    }
}

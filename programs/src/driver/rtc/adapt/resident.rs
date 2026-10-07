//! 一只组等两个源，喂事件、执行动作。

use super::desk;
use crate::dev::rtc;
use env::{HoleDir, Wait};
use programs::driver::rtc::core::frame::Time;
use programs::driver::rtc::core::host::{Host, Ring};
use programs::driver::shared::device::Device;
use programs::driver::shared::fail::Fail;
use programs::unit::rtc::E_RTC;
use ipc::hand::Sender;
use protocol::debug;
use protocol::driver::line;
use env::PAGE_SIZE;
use ::resource::pile::Pile;
use env::pie;
use ::resource::raw::{Hole};

/// 常驻：**一只组等两个源**——门上有请求、线上有投递
/// 两个源都是**事件**：请求是客人推来的，投递是设备自己拉线换来的，故等待没有期限
/// 那只组的成员就是那两枚孔（"就绪"挂进组，"取消息"仍走各自那一手）
/// **这两个源是 rtc 自己的形状**（`uart` 只有一个源、`router` 有三个），故它不收进
/// :——见 programs::driver::mod 那条入库判据
/// `entry` = 本域自己铸的那一枚入口孔（`Context` 只管会话了，故它由 `main` 交过来）。
pub fn run(
    entry: env::PieToken,
    dev: &Device,
    held: line::client::Line,
    host: &mut Host,
) -> Result<(), Fail> {
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_RTC, "desk"))?;
    let entry_hole = Hole::from_raw(entry);
    let lane = held.hole().map_err(|_| Fail::at(E_RTC, "line"))?;
    if pile.attach(entry_hole.token(), HoleDir::Pull).is_err()
        || pile
            .attach(lane, HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(E_RTC, "desk"));
    }

    let view = dev.view();
    // 一问最长那一形是 `Arm`（`Now` 更短，也走得进来）；缓冲给**一页**（余量；孔不预设长度，装不下会答 `Denied` 且手原样）。
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_RTC, "desk"));
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        // 到点才有的说（次序不承担语义，只省一次绕回）。
        match pile.await_(Wait::Forever) {
            Ok(_) => {}
            Err(_) => return Err(Fail::at(E_RTC, "desk")),
        }
        // 门牌是**单槽**：一趟把槽里的都取走。缓冲是一页（见上），故"取不出也丢不掉"
        // 那个状态不存在。
        while let Ok((len, from)) = entry_hole.pull(&mut buf, Wait::POLL) {
            desk::serve(host, view, from, &buf[..len]);
        }
        while held.receive(Wait::POLL).is_ok() {
            // 一次投递 = 设备那一格拉起来了。顺序与 `uart` 同一条道理：**先把设备那一格清干净**
            // （清 `irq_pending`：电平源，不清线就一直挂着），再看那一格到点没有，最后说"排空了"。
            let now = rtc::now(view);
            rtc::clear(view);
            if let Ring::Rang { back, now } = host.ring(now) {
                // 那一声**走 `Sender`**（答那一形：一个时刻）——与客人收它走的是同一张表。
                {
                    let mut tx = Sender::<Time>::from_raw(back);
                    match tx.send(Time::of(now)) {
                        Ok(()) => debug!("rtc: rang n={} now={now}", host.heard()),
                        Err(_) => debug!("rtc: notify failed"),
                    }
                }
                let _ = pie::release(back);
            }
            let _ = held.exhaust();
        }
    }
}

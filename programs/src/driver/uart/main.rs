#![no_std]
#![no_main]

//! serial@10000000 的持有者，兼控制台服务（U 态，见 `driver/uart/mod.rs`）。
//!
//! TX 队列、设备 IRQ 和 RX 空间通知共同驱动循环。RX 满时保留一批，
//! 消费者释放空间后继续交付；该批交付完成后才归还 IRQ。TX 仍可独立推进。

extern crate alloc;
extern crate programs;

/// 起手全在 `adapt::desk`；判定在 `core`/`dev`；本文件只剩流程。
mod adapt;

mod dev;

use crate::dev::uart as device;
use env::{MailCondition, Wait};
use programs::driver::shared::fail::Fail;
use programs::driver::uart::core::frame::{Bytes, DRAIN_MAX};
use programs::unit::uart::E_UART;
use ::resource::pile::Pile;

const MS: usize = 1000;

/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 programs::driver::shared::fail
#[programs::entry]
fn main() -> Result<(), Fail> {
    let mut desk = adapt::desk::start(Wait::AtMost(MS))?;

    // **两个源**：写口那一具架上有人交来的一条字、线上有"设备收来了字节"——组等任意一格
    // （与 `rtc` 那一台同一条判据）。tx 那一格就是**那一枚页**（页上那一位即铃）。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_UART, "desk"))?;
    let lane = desk.line.hole().map_err(|_| Fail::at(E_UART, "line"))?;
    if pile
        .attach(env::Source::Mail { pie: desk.tx.ship(), condition: MailCondition::Signal(env::Bit::FIRST) })
        .is_err()
        || pile.attach(desk.rx_w.source()).is_err()
        || pile
            .attach(env::Source::Mail { pie: lane, condition: MailCondition::Pull })
            .is_err()
    {
        return Err(Fail::at(E_UART, "desk"));
    }
    let view = desk.dev.view();
    let mut raw = [0u8; DRAIN_MAX];
    let mut pending = None;
    loop {
        // ① 写口：人敲的字（读干为止；读空的那一趟把页上那一位应掉）。
        while let Ok(one) = desk.tx_r.recv(Wait::POLL) {
            device::put(view, one.bytes());
        }
        if let Some(batch) = pending.take() {
            if desk.rx_w.send_when_ready(&batch).map_err(|_| Fail::at(E_UART, "rx publish"))? {
                desk.line.exhaust().unwrap();
            } else { pending = Some(batch); }
        }
        if pending.is_none() {
            while desk.line.receive(Wait::POLL).is_ok() {
                let n = device::drain(view, &mut raw);
                if let Some(batch) = Bytes::of(&raw[..n]) {
                    if !desk.rx_w.send_when_ready(&batch).map_err(|_| Fail::at(E_UART, "rx publish"))? {
                        pending = Some(batch); break;
                    }
                }
                // Hold IRQ ownership until its bytes have entered the RX ring.
                desk.line.exhaust().unwrap();
            }
        }
        if pending.is_none() { desk.rx_w.hush(); }
        if pile.await_(Wait::Forever).is_err() { return Err(Fail::at(E_UART, "line gone")); }

    }
}

#![no_std]
#![no_main]

//! serial@10000000 的持有者，兼控制台服务（U 态，见
//! `driver/uart/mod.rs`）。

extern crate alloc;
extern crate programs;

/// 纯功能：交出去的那一批（非空不可表达）
mod core;

mod adapt;

mod dev;

use crate::core::batch::Batch;
use crate::dev::uart as device;
use env::{HoleDir, Wait};
use programs::driver::shared::fail::Fail;
use programs::unit::uart::E_UART;
use runtime::PAGE_SIZE;
use runtime::core::res::pile::Pile;
use runtime::env::mail::HolePie;

/// 它是一块 **Pane**：两枚门牌 `rx` / `tx` 在它下面
const ME: &str = "uart";

const MS: usize = 1000;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里
const DRAIN_MAX: usize = 64;

/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 programs::driver::shared::fail
#[programs::entry]
fn main() -> Result<(), Fail> {
    // 也不写死地址："哪一台是串口"由设备账回答（类 `ns16550a`）。
    let desk = adapt::desk::start(Wait::AtMost(MS))?;

    // **两个源**：写口上有客人交来的一条字、线上有"设备收来了字节"——组等任意一格
    // （与 `rtc` 那一台同一条判据）。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_UART, "desk"))?;
    let lane = desk.line.hole().map_err(|_| Fail::at(E_UART, "line"))?;
    if pile.attach(&desk.tx, HoleDir::Pull).is_err()
        || pile
            .attach(&HolePie::from_token(lane), HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(E_UART, "desk"));
    }
    let view = desk.dev.view();
    // 写口那一页：**余量**——本族一条消息远小于它（与 `rtc` 备缓冲同一手）。
    let mut word: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if word.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_UART, "desk"));
    }
    word.resize(PAGE_SIZE, 0);
    let mut raw = [0u8; DRAIN_MAX];

    loop {
        if pile.await_(Wait::Forever).is_err() {
            return Err(Fail::at(E_UART, "line gone"));
        }
        // FIFO 里排着，多等这一瞬不丢。
        while let Ok((len, _)) = desk.tx.pull(&mut word, Wait::POLL) {
            device::put(view, &word[..len]);
        }
        while desk.line.receive(Wait::POLL).is_ok() {
            let n = device::drain(view, &mut raw);
            // 人（`canonical`）总会回到"取一行"那一格，故等它是有界的。
            if let Some(batch) = Batch::of(&raw, n) {
                desk.ctx.publish(batch.bytes()).unwrap();
            }
            // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 + 把线放回去。
            desk.line.exhaust().unwrap();
        }
    }
}
